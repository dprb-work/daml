//! Cursor observations of the strict data parser, never repaired data admission.
//!
//! Only text reached through the ordinary grammar contributes metadata. Collection
//! scopes and names are observed before an intentional early stop at the cursor;
//! later keys are therefore not included. Strings are scanned by the normal
//! string parser and never produce structural hints.

use super::{Parser, check_input, scan_trivia};
use crate::names::{valid_key, valid_tag};
use crate::{ErrorKind, Limits, ParseError, Span};
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionKind {
    Field,
    Tag,
    Value,
}

#[derive(Debug, Clone)]
pub(crate) struct CompletionContext {
    pub(crate) kind: CompletionKind,
    pub(crate) path: String,
    pub(crate) prefix: Span,
    pub(crate) replace: Span,
    pub(crate) tags: Vec<(String, Span)>,
    pub(crate) fields: Vec<Span>,
}

/// Reports a reliable cursor position without treating unfinished text as data.
/// Syntax failures suppress hints; input, metadata and parser limits remain errors.
pub(crate) fn completion_context(
    source: &str,
    cursor_byte: usize,
    limits: Limits,
) -> Result<Option<CompletionContext>, ParseError> {
    cursor_context::<false>(source, cursor_byte, limits)
}

/// Native grammar observations survive an unfinished enclosing expression.
pub(crate) fn native_completion_context(
    source: &str,
    cursor_byte: usize,
    limits: Limits,
) -> Result<Option<CompletionContext>, ParseError> {
    cursor_context::<true>(source, cursor_byte, limits)
}

fn cursor_context<const NATIVE: bool>(
    source: &str,
    cursor_byte: usize,
    limits: Limits,
) -> Result<Option<CompletionContext>, ParseError> {
    check_input(source, limits)?;
    if cursor_byte > source.len() || !source.is_char_boundary(cursor_byte) {
        let mut start = cursor_byte.min(source.len());
        while !source.is_char_boundary(start) {
            start -= 1;
        }
        let end = source
            .get(start..)
            .and_then(|text| text.chars().next())
            .map_or(start, |ch| start + ch.len_utf8());
        return Err(ParseError::new(ErrorKind::InvalidCharacter, start, end));
    }
    let mut parser = Parser::<true>::new(source, limits);
    parser.editor = Some(Cursor {
        at: cursor_byte,
        blocked: false,
        path: String::new(),
        tags: Vec::new(),
        fields: Vec::new(),
        map_start: 0,
        tag_path_bytes: 0,
        hint: None,
    });
    let parsed = if NATIVE {
        parser.native_definition().map(|_| ())
    } else {
        parser.document().map(|_| ())
    };
    let cursor = parser.editor.take().expect("editor mode has cursor state");
    if let Some((kind, prefix, replace)) = cursor.hint {
        if NATIVE && kind != CompletionKind::Tag {
            return Ok(None);
        }
        let mut fields = cursor.fields;
        drop(fields.drain(..cursor.map_start));
        return Ok(Some(CompletionContext {
            kind,
            path: cursor.path,
            prefix,
            replace,
            tags: cursor.tags,
            fields,
        }));
    }
    match parsed {
        Err(error)
            if matches!(
                error.kind,
                ErrorKind::LimitExceeded | ErrorKind::InvalidLimits
            ) =>
        {
            Err(error)
        }
        _ => Ok(None),
    }
}

/// Reads only the complete initial assertion, even if its body is unfinished.
/// Reserved intrinsic constructors are values, not root schema assertions.
pub(crate) fn root_tag(source: &str, limits: Limits) -> Result<Option<Span>, ParseError> {
    check_input(source, limits)?;
    let mut parser = Parser::<false>::new(source, limits);
    parser.trivia()?;
    if parser.peek() != Some('!') {
        return Ok(None);
    }
    let start = parser.pos;
    parser.pos += 1;
    let tag = parser.tag_name()?;
    if crate::TemporalKind::from_constructor(&source[tag.start..tag.end]).is_some() {
        return Ok(None);
    }
    parser.count_node(start)?;
    Ok(Some(tag))
}

/// Reuse trivia admission after a key; completion must not duplicate its colon.
pub(crate) fn field_has_separator(source: &str, key: Span) -> bool {
    scan_trivia(source, key.end, source.len(), |_| {})
        .is_ok_and(|(end, _)| source.as_bytes().get(end) == Some(&b':'))
}

fn tag_prefix(text: &str) -> bool {
    text.is_empty() || valid_tag(text) || text.strip_suffix('.').is_some_and(valid_tag)
}

pub(super) struct Cursor {
    at: usize,
    blocked: bool,
    path: String,
    tags: Vec<(String, Span)>,
    // Keys of open maps share one stack; only the innermost map is returned.
    fields: Vec<Span>,
    map_start: usize,
    tag_path_bytes: usize,
    hint: Option<(CompletionKind, Span, Span)>,
}

impl Cursor {
    pub(super) fn comment(&mut self, span: Span) {
        // At EOF (or immediately before a newline), the cursor still belongs to
        // the comment. After the physical newline it can be a structural gap.
        self.blocked |= span.start <= self.at && self.at <= span.end;
    }

    fn hint(
        &mut self,
        kind: CompletionKind,
        replace: Span,
        source: &str,
        limits: Limits,
    ) -> Result<bool, ParseError> {
        if self.blocked || self.at < replace.start || self.at > replace.end {
            return Ok(false);
        }
        let prefix = Span {
            start: replace.start,
            end: self.at,
        };
        let text = source
            .get(prefix.start..prefix.end)
            .expect("checked cursor and parser span");
        let valid = match kind {
            CompletionKind::Field => text.is_empty() || valid_key(text),
            CompletionKind::Tag => tag_prefix(text),
            CompletionKind::Value => true,
        };
        if !valid {
            return Ok(false);
        }
        if limits.max_nodes == 0 || replace.end - replace.start > limits.max_scalar_bytes {
            return Err(ParseError::new(
                ErrorKind::LimitExceeded,
                replace.start,
                replace.end,
            ));
        }
        self.hint = Some((kind, prefix, replace));
        Ok(true)
    }

    fn path_budget(&self, length: usize, limits: Limits, span: Span) -> Result<(), ParseError> {
        if length
            .checked_add(self.tag_path_bytes)
            .is_none_or(|bytes| bytes > limits.max_input_bytes)
        {
            return Err(ParseError::new(
                ErrorKind::LimitExceeded,
                span.start,
                span.end,
            ));
        }
        Ok(())
    }

    fn push_key(&mut self, key: Span, source: &str, limits: Limits) -> Result<usize, ParseError> {
        if self.fields.len() >= limits.max_nodes {
            return Err(ParseError::new(
                ErrorKind::LimitExceeded,
                key.start,
                key.end,
            ));
        }
        let name = source
            .get(key.start..key.end)
            .expect("parser-produced key span");
        let length = self
            .path
            .len()
            .checked_add(crate::pointer::segment_len(name))
            .ok_or_else(|| ParseError::new(ErrorKind::LimitExceeded, key.start, key.end))?;
        self.path_budget(length, limits, key)?;
        let parent = self.path.len();
        self.fields.push(key);
        self.path.reserve(length - parent);
        crate::pointer::append(&mut self.path, name);
        Ok(parent)
    }

    fn push_index(&mut self, index: usize, limits: Limits, at: usize) -> Result<usize, ParseError> {
        let digits = index.checked_ilog10().map_or(1, |log| log as usize + 1);
        let length = self
            .path
            .len()
            .checked_add(1 + digits)
            .ok_or_else(|| ParseError::new(ErrorKind::LimitExceeded, at, at))?;
        self.path_budget(length, limits, Span { start: at, end: at })?;
        let parent = self.path.len();
        self.path.reserve(length - parent);
        write!(self.path, "/{index}").expect("writing to a String cannot fail");
        Ok(parent)
    }

    fn tag(&mut self, tag: Span, limits: Limits) -> Result<(), ParseError> {
        if self.tags.len() >= limits.max_nodes {
            return Err(ParseError::new(
                ErrorKind::LimitExceeded,
                tag.start,
                tag.end,
            ));
        }
        let bytes = self
            .tag_path_bytes
            .checked_add(self.path.len())
            .filter(|bytes| {
                bytes
                    .checked_add(self.path.len())
                    .is_some_and(|n| n <= limits.max_input_bytes)
            })
            .ok_or_else(|| ParseError::new(ErrorKind::LimitExceeded, tag.start, tag.end))?;
        self.tags.push((self.path.clone(), tag));
        self.tag_path_bytes = bytes;
        Ok(())
    }
}

impl<const EDITOR: bool, const TOKENS: bool> Parser<'_, EDITOR, TOKENS> {
    pub(super) fn editor_hint(
        &mut self,
        kind: CompletionKind,
        span: Span,
    ) -> Result<(), ParseError> {
        if EDITOR
            && self
                .editor
                .as_mut()
                .expect("editor mode has cursor state")
                .hint(kind, span, self.source, self.limits)?
        {
            // A private control-flow stop, consumed by completion_context only
            // after observing the captured hint. No partial Node is returned.
            return Err(self.error(ErrorKind::ExpectedValue, span.start, span.end));
        }
        Ok(())
    }

    pub(super) fn editor_gap(
        &mut self,
        kind: CompletionKind,
        start: usize,
        separated: bool,
        at_end: bool,
    ) -> Result<(), ParseError> {
        if !EDITOR {
            return Ok(());
        }
        let cursor = self.editor.as_ref().expect("editor mode has cursor state");
        let at = cursor.at;
        if cursor.blocked || at < start || at > self.pos || (at == self.pos && !at_end) {
            return Ok(());
        }
        // A newline after the cursor must not authorize a same-line next entry.
        // Reuse trivia's physical-newline grammar for this bounded prefix.
        let (end, newline) = scan_trivia(self.source, start, at, |_| {})?;
        if end != at || (!separated && !newline) {
            return Ok(());
        }
        self.editor_hint(kind, Span { start: at, end: at })
    }

    pub(super) fn editor_tag(&mut self, tag: Span) -> Result<(), ParseError> {
        if EDITOR {
            self.editor
                .as_mut()
                .expect("editor mode has cursor state")
                .tag(tag, self.limits)?;
        }
        Ok(())
    }

    pub(super) fn editor_enter_map(&mut self) -> (usize, usize) {
        if !EDITOR {
            return (0, 0);
        }
        let cursor = self.editor.as_mut().expect("editor mode has cursor state");
        let scope = (cursor.map_start, cursor.fields.len());
        cursor.map_start = cursor.fields.len();
        scope
    }

    pub(super) fn editor_leave_map(&mut self, scope: (usize, usize)) {
        if EDITOR {
            let cursor = self.editor.as_mut().expect("editor mode has cursor state");
            cursor.fields.truncate(scope.1);
            cursor.map_start = scope.0;
        }
    }

    pub(super) fn editor_field(&mut self, key: Span) -> Result<usize, ParseError> {
        if EDITOR {
            return self
                .editor
                .as_mut()
                .expect("editor mode has cursor state")
                .push_key(key, self.source, self.limits);
        }
        Ok(0)
    }

    pub(super) fn editor_index(&mut self, index: usize) -> Result<usize, ParseError> {
        if EDITOR {
            return self
                .editor
                .as_mut()
                .expect("editor mode has cursor state")
                .push_index(index, self.limits, self.pos);
        }
        Ok(0)
    }

    pub(super) fn editor_leave_path(&mut self, parent: usize) {
        if EDITOR {
            self.editor
                .as_mut()
                .expect("editor mode has cursor state")
                .path
                .truncate(parent);
        }
    }
}
