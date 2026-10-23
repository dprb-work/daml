//! Native type definitions and explicitly selected directory bundles.

mod bundle;
mod compiler;
mod error;
mod format;
mod syntax;

use crate::{Node, Span};

pub use bundle::{BUNDLE_MANIFEST, Bundle};
pub use error::{NativeError, NativeErrorKind};
pub use format::format_definition;
pub use syntax::{parse_definition, parse_definition_with_limits};

/// A native definition's original source and checked syntax.
///
/// Parsing preserves reference names without loading bundles or validating data.
#[derive(Debug, PartialEq)]
pub struct Definition {
    pub(crate) source: String,
    pub(crate) root: Expression,
}

impl Definition {
    /// Returns the original source, including comments and literal spelling.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the original-source range of the complete expression.
    pub fn span(&self) -> Span {
        self.root.span
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Expression {
    pub(crate) span: Span,
    pub(crate) kind: ExpressionKind,
    pub(crate) default: DefaultValue,
    pub(crate) assignment: Option<Span>,
}

impl Expression {
    pub(crate) fn effective_default(&self) -> &DefaultValue {
        if matches!(self.default, DefaultValue::Inherited)
            && let ExpressionKind::Group(inner) | ExpressionKind::Closed(inner) = &self.kind
        {
            return inner.effective_default();
        }
        &self.default
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum ExpressionKind {
    Literal(Node),
    Regex(Span),
    Tagged {
        name: Span,
        body: Option<Box<Expression>>,
    },
    Record(Vec<Member>),
    Tuple(Vec<Expression>),
    Group(Box<Expression>),
    Union(Vec<Expression>),
    Intersection(Vec<Expression>),
    Closed(Box<Expression>),
}

#[derive(Debug, PartialEq)]
pub(crate) struct Member {
    pub(crate) span: Span,
    pub(crate) key: Span,
    pub(crate) optional: bool,
    pub(crate) constraint: Expression,
}

#[derive(Debug, PartialEq)]
pub(crate) enum DefaultValue {
    Inherited,
    Unset,
    Value(Node),
}
