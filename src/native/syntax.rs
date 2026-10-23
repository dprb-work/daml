use crate::native::Definition;
use crate::{Limits, ParseError};
use std::borrow::Cow;

/// Parses one complete native definition or ordinary DAML data expression.
///
/// References remain unresolved. The definition retains its original source so
/// every syntax span continues to address the caller's spelling.
///
/// # Errors
///
/// Returns a structured syntax or resource-limit error.
pub fn parse_definition<'a>(source: impl Into<Cow<'a, str>>) -> Result<Definition, ParseError> {
    parse_definition_with_limits(source, Limits::default())
}

/// Parses one complete native definition with explicit resource limits.
///
/// `max_depth` is also bounded by the parser's hard ceiling of 128. Source spans
/// always refer to the original UTF-8 input.
///
/// # Errors
///
/// Returns syntax/resource errors as [`parse_definition`] does, or
/// [`crate::ErrorKind::InvalidLimits`] when the requested depth exceeds 128.
pub fn parse_definition_with_limits<'a>(
    source: impl Into<Cow<'a, str>>,
    limits: Limits,
) -> Result<Definition, ParseError> {
    let source = source.into();
    let root = crate::parser::native::parse_definition(source.as_ref(), limits)?;
    Ok(Definition {
        source: source.into_owned(),
        root,
    })
}
