#[cfg(feature = "lsp")]
pub(crate) mod editor;
pub(crate) mod native;
pub(crate) mod tokens;
use crate::names::{valid_key, valid_tag};
use crate::source::line_end;
use crate::{Entry, ErrorKind, Limits, Node, ParseError, Span, Value};
use std::collections::HashSet;
use tokens::Kind as TokenKind;

const HARD_MAX_DEPTH: usize = 128;

pub(crate) fn check_input(source: &str, limits: Limits) -> Result<(), ParseError> {
    if limits.max_depth > HARD_MAX_DEPTH {
        return Err(ParseError::new(ErrorKind::InvalidLimits, 0, 0));
    }
    if source.len() > limits.max_input_bytes {
        let mut boundary = limits.max_input_bytes.min(source.len());
        while !source.is_char_boundary(boundary) {
            boundary -= 1;
        }
        return Err(ParseError::new(
            ErrorKind::LimitExceeded,
            boundary,
            source.len(),
        ));
    }
    Ok(())
}
/// Scan the shared whitespace/comment grammar without allocating trivia for parsing.
pub(crate) fn scan_trivia(
    source: &str,
    start: usize,
    end: usize,
    mut comment: impl FnMut(Span),
) -> Result<(usize, bool), ParseError> {
    let source = &source[..end];
    let mut pos = start;
    let mut newline = false;
    while let Some(ch) = source[pos..].chars().next() {
        match ch {
            ' ' | '\t' => pos += 1,
            '\r' | '\n' => {
                pos = line_end(source.as_bytes(), pos).expect("matched physical line ending");
                newline = true;
            }
            '#' => {
                let start = pos;
                while let Some(ch) = source[pos..].chars().next() {
                    if matches!(ch, '\r' | '\n') {
                        break;
                    }
                    if ch == '\u{feff}' || (ch.is_ascii_control() && ch != '\t') {
                        return Err(ParseError::new(
                            ErrorKind::InvalidCharacter,
                            pos,
                            pos + ch.len_utf8(),
                        ));
                    }
                    pos += ch.len_utf8();
                }
                comment(Span { start, end: pos });
            }
            '\u{feff}' => {
                return Err(ParseError::new(
                    ErrorKind::InvalidCharacter,
                    pos,
                    pos + ch.len_utf8(),
                ));
            }
            _ => break,
        }
    }
    Ok((pos, newline))
}

pub(crate) fn parse(source: &str, limits: Limits) -> Result<Node, ParseError> {
    check_input(source, limits)?;
    Parser::<false>::new(source, limits).document()
}

struct Parser<'a, const EDITOR: bool = false, const TOKENS: bool = false> {
    source: &'a str,
    pos: usize,
    limits: Limits,
    nodes: usize,
    #[cfg(feature = "lsp")]
    editor: Option<editor::Cursor>,
    #[cfg(feature = "lsp")]
    tokens: Vec<tokens::Token>,
}

impl<'a, const EDITOR: bool, const TOKENS: bool> Parser<'a, EDITOR, TOKENS> {
    fn new(source: &'a str, limits: Limits) -> Self {
        Self {
            source,
            pos: usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8(),
            limits,
            nodes: 0,
            #[cfg(feature = "lsp")]
            editor: None,
            #[cfg(feature = "lsp")]
            tokens: Vec::new(),
        }
    }

    fn document(&mut self) -> Result<Node, ParseError> {
        #[cfg(feature = "lsp")]
        let gap_start = self.pos;
        self.trivia()?;
        #[cfg(feature = "lsp")]
        self.editor_gap(
            editor::CompletionKind::Value,
            gap_start,
            true,
            self.peek().is_none(),
        )?;
        if self.pos == self.source.len() {
            return Err(self.error(ErrorKind::ExpectedValue, self.pos, self.pos));
        }
        let root = self.value(0)?;
        self.trivia()?;
        if self.pos != self.source.len() {
            return Err(self.error(ErrorKind::TrailingInput, self.pos, self.char_end(self.pos)));
        }
        Ok(root)
    }

    fn error(&self, kind: ErrorKind, start: usize, end: usize) -> ParseError {
        ParseError::new(kind, start, end)
    }

    fn char_end(&self, at: usize) -> usize {
        if at >= self.source.len() {
            at
        } else {
            at + self.source[at..].chars().next().unwrap().len_utf8()
        }
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos..)?.chars().next()
    }

    fn trivia(&mut self) -> Result<bool, ParseError> {
        #[cfg(feature = "lsp")]
        if TOKENS {
            return self.token_trivia();
        }
        #[cfg(feature = "lsp")]
        if EDITOR {
            let cursor = self.editor.as_mut().expect("editor mode has cursor state");
            let (end, newline) = scan_trivia(self.source, self.pos, self.source.len(), |span| {
                cursor.comment(span)
            })?;
            self.pos = end;
            return Ok(newline);
        }
        let (end, newline) = scan_trivia(self.source, self.pos, self.source.len(), |_| {})?;
        self.pos = end;
        Ok(newline)
    }

    fn count_node(&mut self, start: usize) -> Result<(), ParseError> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error(ErrorKind::LimitExceeded, start, self.char_end(start)));
        }
        self.nodes += 1;
        Ok(())
    }

    fn value(&mut self, parent_depth: usize) -> Result<Node, ParseError> {
        #[cfg(feature = "lsp")]
        let gap_start = self.pos;
        self.trivia()?;
        #[cfg(feature = "lsp")]
        self.editor_gap(
            editor::CompletionKind::Value,
            gap_start,
            true,
            matches!(self.peek(), None | Some('}' | ']')),
        )?;
        let start = self.pos;
        let tag = if self.peek() == Some('!') {
            self.operator()?;
            let tag = self.tag_name()?;
            let tag_end = tag.end;
            self.horizontal();
            if let Some(kind) =
                crate::TemporalKind::from_constructor(&self.source[tag.start..tag.end])
            {
                return self.temporal_literal(start, None, kind);
            }
            #[cfg(feature = "lsp")]
            self.editor_tag(tag)?;
            if self.peek() == Some('!') {
                let intrinsic_start = self.pos + 1;
                let end = self.source[intrinsic_start..]
                    .find(|ch: char| !is_name_char(ch))
                    .map_or(self.source.len(), |n| intrinsic_start + n);
                if let Some(kind) =
                    crate::TemporalKind::from_constructor(&self.source[intrinsic_start..end])
                {
                    self.operator()?;
                    self.pos = end;
                    self.observe_name(
                        Span {
                            start: intrinsic_start,
                            end,
                        },
                        false,
                    )?;
                    self.horizontal();
                    return self.temporal_literal(start, Some(tag), kind);
                }
                return Err(self.error(ErrorKind::InvalidTag, self.pos, self.char_end(self.pos)));
            }
            let next = self.peek();
            if next.is_none() || matches!(next, Some('}' | ']' | ')' | '#' | '\r' | '\n')) {
                if parent_depth + 1 > self.limits.max_depth || parent_depth + 1 > HARD_MAX_DEPTH {
                    return Err(self.error(ErrorKind::LimitExceeded, start, tag_end));
                }
                self.rewind(tag_end);
                self.count_node(start)?;
                return Ok(Node {
                    span: Span {
                        start,
                        end: tag_end,
                    },
                    tag: Some(tag),
                    value: Value::Mapping(Vec::new()),
                });
            }
            Some(tag)
        } else {
            None
        };
        self.count_node(start)?;
        let scalar_start = self.pos;
        let value = match self.peek() {
            Some('{') => self.mapping(parent_depth + 1)?,
            Some('[') => self.sequence(parent_depth + 1)?,
            Some('"') | Some('\'')
                if self.source[self.pos..].starts_with("\"\"\"")
                    || self.source[self.pos..].starts_with("'''") =>
            {
                let (string, end) = self.quoted(false)?;
                self.pos = end;
                Value::String(string)
            }
            Some('"') => {
                let (string, end) = self.quoted(false)?;
                self.pos = end;
                Value::String(string)
            }
            Some('\'') => {
                return Err(self.error(
                    ErrorKind::UnexpectedToken,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            Some('t') if self.keyword("true") => {
                self.pos += 4;
                self.observe(
                    Span {
                        start: scalar_start,
                        end: self.pos,
                    },
                    TokenKind::Keyword,
                )?;
                Value::Bool(true)
            }
            Some('f') if self.keyword("false") => {
                self.pos += 5;
                self.observe(
                    Span {
                        start: scalar_start,
                        end: self.pos,
                    },
                    TokenKind::Keyword,
                )?;
                Value::Bool(false)
            }
            Some('n') if self.keyword("null") => {
                self.pos += 4;
                self.observe(
                    Span {
                        start: scalar_start,
                        end: self.pos,
                    },
                    TokenKind::Keyword,
                )?;
                Value::Null
            }
            Some('-' | '.' | '0'..='9') => self.number()?,
            Some(_) => {
                return Err(self.error(
                    ErrorKind::ExpectedValue,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            None => return Err(self.error(ErrorKind::ExpectedValue, self.pos, self.pos)),
        };
        if matches!(value, Value::Number(_) | Value::Bool(_) | Value::Null)
            && self.pos - scalar_start > self.limits.max_scalar_bytes
        {
            return Err(self.error(ErrorKind::LimitExceeded, scalar_start, self.pos));
        }
        #[cfg(feature = "lsp")]
        if matches!(value, Value::Number(_) | Value::Bool(_) | Value::Null) {
            self.editor_hint(
                editor::CompletionKind::Value,
                Span {
                    start: scalar_start,
                    end: self.pos,
                },
            )?;
        }
        Ok(Node {
            span: Span {
                start,
                end: self.pos,
            },
            tag,
            value,
        })
    }

    // Shared by strict data parsing and root-assertion inspection. Cursor mode
    // observes the original name before validation, without admitting it.
    fn tag_name(&mut self) -> Result<Span, ParseError> {
        let start = self.pos;
        while self.peek().is_some_and(is_name_char) {
            self.pos += self.peek().unwrap().len_utf8();
        }
        if self.peek().is_some_and(is_forbidden_token_char) {
            return Err(self.error(
                ErrorKind::InvalidCharacter,
                self.pos,
                self.char_end(self.pos),
            ));
        }
        if self.pos - start > self.limits.max_scalar_bytes {
            return Err(self.error(ErrorKind::LimitExceeded, start, self.pos));
        }
        let span = Span {
            start,
            end: self.pos,
        };
        #[cfg(feature = "lsp")]
        self.editor_hint(editor::CompletionKind::Tag, span)?;
        if self.pos == start || !valid_tag(&self.source[start..self.pos]) {
            return Err(self.error(ErrorKind::InvalidTag, start, self.pos));
        }
        self.observe_name(span, false)?;
        Ok(span)
    }

    fn temporal_literal(
        &mut self,
        start: usize,
        tag: Option<Span>,
        kind: crate::TemporalKind,
    ) -> Result<Node, ParseError> {
        self.count_node(start)?;
        let payload_start = self.pos;
        let invalid = if kind == crate::TemporalKind::Cron {
            ErrorKind::InvalidCron
        } else {
            ErrorKind::InvalidTemporal
        };
        if !matches!(self.peek(), Some('"' | '\'')) {
            return Err(self.error(invalid, start, self.char_end(self.pos)));
        }
        let (text, end) = self.quoted(false).map_err(|error| {
            if error.kind == ErrorKind::LimitExceeded {
                error
            } else {
                self.error(invalid, payload_start, error.span.end)
            }
        })?;
        self.pos = end;
        let value = crate::ParsedTemporal::parse(kind, text, self.source)
            .map_err(|_| self.error(invalid, payload_start, end))?;
        if value
            .value(self.source)
            .expect("admitted source")
            .as_str()
            .len()
            .saturating_add(2)
            > self.limits.max_scalar_bytes
        {
            return Err(self.error(ErrorKind::LimitExceeded, payload_start, end));
        }
        Ok(Node {
            span: Span { start, end },
            tag,
            value: Value::Temporal(value),
        })
    }

    fn horizontal(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
        self.pos != start
    }

    fn keyword(&self, word: &str) -> bool {
        self.source[self.pos..].starts_with(word)
            && self.source[self.pos + word.len()..]
                .chars()
                .next()
                .is_none_or(|c| !is_name_char(c))
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        match self.peek() {
            Some('0') => {
                self.pos += 1;
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    return Err(self.error(
                        ErrorKind::InvalidNumber,
                        start,
                        self.char_end(self.pos),
                    ));
                }
            }
            Some('1'..='9') => {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error(ErrorKind::InvalidNumber, start, self.char_end(self.pos))),
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            let digits = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == digits {
                return Err(self.error(ErrorKind::InvalidNumber, start, self.pos));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            let digits = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == digits {
                return Err(self.error(ErrorKind::InvalidNumber, start, self.pos));
            }
        }
        if self.pos - start > self.limits.max_scalar_bytes {
            return Err(self.error(ErrorKind::LimitExceeded, start, self.pos));
        }
        if self.peek().is_some_and(is_name_char) {
            return Err(self.error(ErrorKind::InvalidNumber, start, self.char_end(self.pos)));
        }
        self.observe(
            Span {
                start,
                end: self.pos,
            },
            TokenKind::Number,
        )?;
        Ok(Value::Number(Span {
            start,
            end: self.pos,
        }))
    }

    fn mapping(&mut self, depth: usize) -> Result<Value, ParseError> {
        let open = self.pos;
        if depth > self.limits.max_depth || depth > HARD_MAX_DEPTH {
            return Err(self.error(ErrorKind::LimitExceeded, open, self.char_end(open)));
        }
        self.operator()?;
        let mut entries = Vec::new();
        let mut keys = HashSet::new();
        let mut separated = true;
        #[cfg(feature = "lsp")]
        let map_scope = self.editor_enter_map();
        loop {
            #[cfg(feature = "lsp")]
            let gap_start = self.pos;
            let newline = self.trivia()?;
            #[cfg(feature = "lsp")]
            self.editor_gap(
                editor::CompletionKind::Field,
                gap_start,
                separated,
                matches!(self.peek(), None | Some('}')),
            )?;
            if self.peek() == Some('}') {
                self.operator()?;
                #[cfg(feature = "lsp")]
                self.editor_leave_map(map_scope);
                return Ok(Value::Mapping(entries));
            }
            if self.pos == self.source.len() {
                return Err(self.error(ErrorKind::UnterminatedCollection, open, self.pos));
            }
            if !separated && !newline {
                return Err(self.error(
                    ErrorKind::ExpectedSeparator,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            let key_start = self.pos;
            if self.peek().is_some_and(is_forbidden_token_char) {
                return Err(self.error(
                    ErrorKind::InvalidCharacter,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            while self.peek().is_some_and(is_name_char) {
                self.pos += self.peek().unwrap().len_utf8();
            }
            if self.peek().is_some_and(is_forbidden_token_char) {
                return Err(self.error(
                    ErrorKind::InvalidCharacter,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            if self.pos == key_start {
                return Err(self.error(ErrorKind::ExpectedKey, self.pos, self.char_end(self.pos)));
            }
            if self.pos - key_start > self.limits.max_scalar_bytes {
                return Err(self.error(ErrorKind::LimitExceeded, key_start, self.pos));
            }
            #[cfg(feature = "lsp")]
            self.editor_hint(
                editor::CompletionKind::Field,
                Span {
                    start: key_start,
                    end: self.pos,
                },
            )?;
            if !valid_key(&self.source[key_start..self.pos]) {
                return Err(self.error(ErrorKind::ExpectedKey, key_start, self.pos));
            }
            let key = Span {
                start: key_start,
                end: self.pos,
            };
            self.observe(key, TokenKind::Property)?;
            if !keys.insert(&self.source[key_start..self.pos]) {
                return Err(self.error(ErrorKind::DuplicateKey, key_start, self.pos));
            }
            self.horizontal();
            if self.peek() != Some(':') {
                return Err(self.error(
                    ErrorKind::ExpectedColon,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            self.operator()?;
            #[cfg(feature = "lsp")]
            let parent_path = self.editor_field(key)?;
            let value = self.value(depth)?;
            #[cfg(feature = "lsp")]
            self.editor_leave_path(parent_path);
            entries.push(Entry { key, value });
            separated = false;
        }
    }

    fn sequence(&mut self, depth: usize) -> Result<Value, ParseError> {
        let open = self.pos;
        if depth > self.limits.max_depth || depth > HARD_MAX_DEPTH {
            return Err(self.error(ErrorKind::LimitExceeded, open, self.char_end(open)));
        }
        self.operator()?;
        let mut items = Vec::new();
        let mut separated = true;
        loop {
            #[cfg(feature = "lsp")]
            let gap_start = self.pos;
            let newline = self.trivia()?;
            #[cfg(feature = "lsp")]
            let parent_path = self.editor_index(items.len())?;
            #[cfg(feature = "lsp")]
            self.editor_gap(
                editor::CompletionKind::Value,
                gap_start,
                separated,
                matches!(self.peek(), None | Some(']')),
            )?;
            if self.peek() == Some(']') {
                self.operator()?;
                #[cfg(feature = "lsp")]
                self.editor_leave_path(parent_path);
                return Ok(Value::Sequence(items));
            }
            if self.pos == self.source.len() {
                return Err(self.error(ErrorKind::UnterminatedCollection, open, self.pos));
            }
            if !separated && !newline {
                return Err(self.error(
                    ErrorKind::ExpectedSeparator,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            items.push(self.value(depth)?);
            #[cfg(feature = "lsp")]
            self.editor_leave_path(parent_path);
            separated = false;
        }
    }
}

fn is_forbidden_token_char(ch: char) -> bool {
    ch == '\u{feff}' || (ch.is_ascii_control() && !matches!(ch, '\t' | '\r' | '\n'))
}

fn is_name_char(ch: char) -> bool {
    !ch.is_whitespace()
        && !is_forbidden_token_char(ch)
        && !matches!(
            ch,
            '{' | '}'
                | '['
                | ']'
                | '('
                | ')'
                | ':'
                | ','
                | '#'
                | '"'
                | '\''
                | '!'
                | '&'
                | '*'
                | '|'
                | '>'
                | '%'
                | '\\'
        )
}
