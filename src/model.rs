use std::fmt;

/// A half-open byte range in the original UTF-8 source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// First byte of the range.
    pub start: usize,
    /// First byte after the range.
    pub end: usize,
}

/// A string that either uses unchanged source content or owns decoded text.
#[derive(Debug, PartialEq)]
pub enum StringValue {
    /// Unchanged content, excluding its source delimiters.
    Source(Span),
    /// Text changed by escapes, dedent, continuation, or wrapping.
    Decoded(String),
}

/// A parsed value, independent of any schema or application constructor.
#[derive(Debug, PartialEq)]
pub enum Value {
    /// The explicit `null` value.
    Null,
    /// An explicit lowercase boolean.
    Bool(bool),
    /// A JSON-style decimal number retained without precision loss.
    Number(Span),
    /// Ordinary, dedented literal, or wrapped text.
    String(StringValue),
    /// An admitted intrinsic temporal literal, not an ordinary string or assertion.
    Temporal(crate::ParsedTemporal),
    /// Ordered sequence entries.
    Sequence(Vec<Node>),
    /// Ordered mapping entries with unique unquoted keys.
    Mapping(Vec<Entry>),
}

/// A value with its optional asserted tag and original source location.
#[derive(Debug, PartialEq)]
pub struct Node {
    /// Entire node, including any tag and string delimiters.
    pub span: Span,
    /// Tag name bytes, excluding the leading `!`.
    pub tag: Option<Span>,
    /// The syntactic value; tags are not resolved during parsing.
    pub value: Value,
}

/// An unquoted mapping key and its value.
#[derive(Debug, PartialEq)]
pub struct Entry {
    /// Exact key spelling in the original source.
    pub key: Span,
    /// Value belonging to this key, regardless of presentation indentation.
    pub value: Node,
}

/// An owned source and its parsed root, without self-referential borrows.
#[derive(Debug, PartialEq)]
pub struct Document {
    pub(crate) source: String,
    pub(crate) root: Node,
}

impl Document {
    /// Returns the complete original source, including comments and whitespace.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the single root value.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Resolves a source byte range, returning `None` for invalid UTF-8 boundaries.
    pub fn slice(&self, span: Span) -> Option<&str> {
        self.source.get(span.start..span.end)
    }

    /// Returns decoded string content, or `None` for a non-string/invalid range.
    ///
    /// Pass nodes belonging to this document; spans refer to this source only.
    pub fn string<'a>(&'a self, node: &'a Node) -> Option<&'a str> {
        match &node.value {
            Value::String(StringValue::Source(span)) => self.slice(*span),
            Value::String(StringValue::Decoded(text)) => Some(text),
            _ => None,
        }
    }

    /// Borrows an admitted canonical temporal value without reparsing its payload.
    /// Pass nodes belonging to this document, as with [`Self::string`].
    pub fn temporal<'a>(&'a self, node: &'a Node) -> Option<crate::TemporalValue<'a>> {
        match &node.value {
            Value::Temporal(value) => value.value(&self.source),
            _ => None,
        }
    }

    /// Returns the asserted tag name without `!`, without resolving its schema.
    pub fn tag(&self, node: &Node) -> Option<&str> {
        self.slice(node.tag?)
    }

    /// Finds an exact literal key in a mapping belonging to this document.
    ///
    /// Dots and slashes in a name never imply traversal. This does not convert
    /// the ordered mapping into a second stored index.
    pub fn get<'a>(&self, node: &'a Node, key: &str) -> Option<&'a Node> {
        let Value::Mapping(entries) = &node.value else {
            return None;
        };
        entries
            .iter()
            .find(|entry| self.slice(entry.key) == Some(key))
            .map(|entry| &entry.value)
    }
}

/// Resource bounds applied before or during parsing.
///
/// Depth counts nested mappings/sequences, not tags or scalar leaves. The
/// implementation has a hard nesting ceiling of 128 even with custom limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum original UTF-8 source bytes (default 8 MiB).
    pub max_input_bytes: usize,
    /// Maximum bytes in a scalar token or key/tag name (default 1 MiB).
    pub max_scalar_bytes: usize,
    /// Maximum number of value nodes, including the root (default 100,000).
    pub max_nodes: usize,
    /// Maximum mapping/sequence nesting (default and hard ceiling 128).
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 8 * 1024 * 1024,
            max_scalar_bytes: 1024 * 1024,
            max_nodes: 100_000,
            max_depth: 128,
        }
    }
}

/// A machine-readable syntax or resource failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// A value is missing or has no supported explicit spelling.
    ExpectedValue,
    /// A mapping key is missing or quoted.
    ExpectedKey,
    /// A key is not followed by its colon.
    ExpectedColon,
    /// Collection entries are not separated by a newline.
    ExpectedSeparator,
    /// An unexpected or unsupported syntax token was encountered.
    UnexpectedToken,
    /// Nontrivia content follows the root value.
    TrailingInput,
    /// A mapping repeats an exact key spelling.
    DuplicateKey,
    /// A numeric token does not follow the decimal grammar.
    InvalidNumber,
    /// An escape or continuation is invalid.
    InvalidEscape,
    /// A string delimiter or raw character violates its string form.
    InvalidString,
    /// An intrinsic temporal constructor has a missing or invalid ISO payload.
    InvalidTemporal,
    /// A string has no closing delimiter.
    UnterminatedString,
    /// A mapping or sequence has no closing delimiter.
    UnterminatedCollection,
    /// A tag is empty or another tag is stacked on the same node.
    InvalidTag,
    /// A source character is not permitted at its position.
    InvalidCharacter,
    /// An input, scalar, node, or nesting bound was exceeded.
    LimitExceeded,
    /// Custom limits exceed the implementation's hard nesting ceiling.
    InvalidLimits,
}

/// A fail-fast error whose span refers to the original source, not decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Structured failure category.
    pub kind: ErrorKind,
    /// Original byte location; EOF errors may have an empty span.
    pub span: Span,
}

impl ParseError {
    pub(crate) fn new(kind: ErrorKind, start: usize, end: usize) -> Self {
        Self {
            kind,
            span: Span { start, end },
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self.kind {
            ErrorKind::ExpectedValue => "expected an explicit value",
            ErrorKind::ExpectedKey => "expected an unquoted key",
            ErrorKind::ExpectedColon => "expected ':' after key",
            ErrorKind::ExpectedSeparator => "expected a newline between entries",
            ErrorKind::UnexpectedToken => "unexpected syntax token",
            ErrorKind::TrailingInput => "unexpected content after root value",
            ErrorKind::DuplicateKey => "duplicate mapping key",
            ErrorKind::InvalidNumber => "invalid decimal number",
            ErrorKind::InvalidEscape => "invalid escape or continuation",
            ErrorKind::InvalidString => "invalid string syntax",
            ErrorKind::InvalidTemporal => "invalid temporal literal",
            ErrorKind::UnterminatedString => "unterminated string",
            ErrorKind::UnterminatedCollection => "unterminated collection",
            ErrorKind::InvalidTag => "invalid or stacked tag",
            ErrorKind::InvalidCharacter => "invalid source character",
            ErrorKind::LimitExceeded => "parser resource limit exceeded",
            ErrorKind::InvalidLimits => "nesting limit exceeds hard ceiling of 128",
        };
        write!(
            formatter,
            "{description} at bytes {}..{}",
            self.span.start, self.span.end
        )
    }
}

impl std::error::Error for ParseError {}
