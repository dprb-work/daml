//! Source observations made only by the token-request parser specialization.

use super::Parser;
use crate::{ErrorKind, ParseError, Span, StringValue};

// Discriminants are the standard legend's indices, not editor-specific scopes.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Property,
    #[cfg(feature = "lsp")]
    Namespace,
    #[cfg(feature = "lsp")]
    Type,
    String,
    Number,
    Keyword,
    #[cfg(feature = "lsp")]
    Comment,
    Regexp,
    Operator,
}

#[cfg(feature = "lsp")]
pub(crate) const MAX_TOKENS: usize = 100_000;

#[cfg(feature = "lsp")]
pub(crate) struct Token {
    pub(crate) span: Span,
    pub(crate) kind: Kind,
    pub(crate) default_library: bool,
}

#[cfg(feature = "lsp")]
pub(crate) fn collect(source: &str, data: bool) -> Result<Vec<Token>, ParseError> {
    let limits = crate::Limits::default();
    super::check_input(source, limits)?;
    let mut parser = Parser::<false, true>::new(source, limits);
    let parsed = if data {
        parser.document().map(|_| ())
    } else {
        parser.native_definition().map(|_| ())
    };
    if let Err(error) = parsed
        && matches!(
            error.kind,
            ErrorKind::LimitExceeded | ErrorKind::InvalidLimits
        )
    {
        return Err(error);
    }
    // Syntax errors leave only observations reached by the real grammar. No
    // repair, suffix scan, or partial data model is exposed to callers.
    Ok(parser.tokens)
}

#[cfg(feature = "lsp")]
fn push(
    tokens: &mut Vec<Token>,
    source: &str,
    span: Span,
    kind: Kind,
    default_library: bool,
) -> Result<(), ParseError> {
    let mut start = span.start;
    let mut pos = start;
    loop {
        let newline = crate::source::line_end(source.as_bytes(), pos);
        if pos == span.end || newline.is_some() {
            if start < pos {
                if tokens.len() == MAX_TOKENS {
                    return Err(ParseError::new(ErrorKind::LimitExceeded, start, pos));
                }
                tokens.push(Token {
                    span: Span { start, end: pos },
                    kind,
                    default_library,
                });
            }
            if pos == span.end {
                return Ok(());
            }
            pos = newline.expect("matched physical line ending");
            start = pos;
        } else {
            pos += source[pos..]
                .chars()
                .next()
                .expect("before span end")
                .len_utf8();
        }
    }
}

impl<const EDITOR: bool, const TOKENS: bool> Parser<'_, EDITOR, TOKENS> {
    pub(super) fn observe(&mut self, span: Span, kind: Kind) -> Result<(), ParseError> {
        #[cfg(feature = "lsp")]
        if TOKENS {
            return push(&mut self.tokens, self.source, span, kind, false);
        }
        let _ = (span, kind);
        Ok(())
    }

    pub(super) fn operator(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        self.pos += 1; // Called only after the grammar recognizes ASCII punctuation.
        self.observe(
            Span {
                start,
                end: self.pos,
            },
            Kind::Operator,
        )
    }

    pub(super) fn rewind(&mut self, pos: usize) {
        #[cfg(feature = "lsp")]
        if TOKENS {
            // Lookahead can have consumed trivia, or a native temporal atom can
            // restart in the data grammar. Roll observations back with the input.
            while self.tokens.last().is_some_and(|token| token.span.end > pos) {
                self.tokens.pop();
            }
        }
        self.pos = pos;
    }

    pub(super) fn observe_name(
        &mut self,
        span: Span,
        builtin_constructor: bool,
    ) -> Result<(), ParseError> {
        #[cfg(feature = "lsp")]
        if TOKENS {
            let text = &self.source[span.start..span.end];
            let default_library =
                builtin_constructor || crate::native::EDITOR_BUILTINS.contains(&text);
            let mut start = span.start;
            for part in text.split('.') {
                let end = start + part.len();
                let kind = if end == span.end {
                    Kind::Type
                } else {
                    Kind::Namespace
                };
                push(
                    &mut self.tokens,
                    self.source,
                    Span { start, end },
                    kind,
                    default_library,
                )?;
                if end < span.end {
                    push(
                        &mut self.tokens,
                        self.source,
                        Span {
                            start: end,
                            end: end + 1,
                        },
                        Kind::Operator,
                        false,
                    )?;
                }
                start = end + 1;
            }
        }
        let _ = (span, builtin_constructor);
        Ok(())
    }

    pub(super) fn quoted(&mut self, regex: bool) -> Result<(StringValue, usize), ParseError> {
        let start = self.pos;
        let result = if regex {
            crate::strings::scan_regex(self.source, start, self.limits.max_scalar_bytes)
        } else {
            crate::strings::scan(self.source, start, self.limits.max_scalar_bytes)
        };
        let kind = if regex { Kind::Regexp } else { Kind::String };
        match result {
            Ok((value, end)) => {
                self.observe(Span { start, end }, kind)?;
                Ok((value, end))
            }
            Err(error) => {
                if TOKENS && error.kind != ErrorKind::LimitExceeded {
                    let end = if error.kind == ErrorKind::UnterminatedString {
                        error.span.end
                    } else {
                        error.span.start.max(start)
                    };
                    if end - start > self.limits.max_scalar_bytes {
                        return Err(self.error(ErrorKind::LimitExceeded, start, end));
                    }
                    self.observe(Span { start, end }, kind)?;
                }
                Err(error)
            }
        }
    }

    #[cfg(feature = "lsp")]
    pub(super) fn token_trivia(&mut self) -> Result<bool, ParseError> {
        let mut failure = None;
        let scanned = super::scan_trivia(self.source, self.pos, self.source.len(), |span| {
            if failure.is_none() {
                failure = push(&mut self.tokens, self.source, span, Kind::Comment, false).err();
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        let (end, newline) = scanned?;
        self.pos = end;
        Ok(newline)
    }
}
