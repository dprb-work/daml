pub(crate) mod native;
use crate::names::{valid_key, valid_tag};
use crate::source::line_end;
use crate::{Entry, ErrorKind, Limits, Node, ParseError, Span, Value};
use std::collections::HashSet;

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
    let mut parser = Parser {
        source,
        pos: usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8(),
        limits,
        nodes: 0,
    };
    parser.trivia()?;
    if parser.pos == source.len() {
        return Err(parser.error(ErrorKind::ExpectedValue, parser.pos, parser.pos));
    }
    let root = parser.value(0)?;
    parser.trivia()?;
    if parser.pos != source.len() {
        return Err(parser.error(
            ErrorKind::TrailingInput,
            parser.pos,
            parser.char_end(parser.pos),
        ));
    }
    Ok(root)
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
    limits: Limits,
    nodes: usize,
}

impl Parser<'_> {
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
        self.trivia()?;
        let start = self.pos;
        let tag = if self.peek() == Some('!') {
            self.pos += 1;
            let tag_start = self.pos;
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
            if self.pos == tag_start || self.pos - tag_start > self.limits.max_scalar_bytes {
                return Err(self.error(
                    if self.pos == tag_start {
                        ErrorKind::InvalidTag
                    } else {
                        ErrorKind::LimitExceeded
                    },
                    tag_start,
                    self.pos,
                ));
            }
            if !valid_tag(&self.source[tag_start..self.pos]) {
                return Err(self.error(ErrorKind::InvalidTag, tag_start, self.pos));
            }
            let tag_end = self.pos;
            let tag = Span {
                start: tag_start,
                end: tag_end,
            };
            self.horizontal();
            if let Some(kind) =
                crate::TemporalKind::from_constructor(&self.source[tag.start..tag.end])
            {
                return self.temporal_literal(start, None, kind);
            }
            if self.peek() == Some('!') {
                let intrinsic_start = self.pos + 1;
                let end = self.source[intrinsic_start..]
                    .find(|ch: char| !is_name_char(ch))
                    .map_or(self.source.len(), |n| intrinsic_start + n);
                if let Some(kind) =
                    crate::TemporalKind::from_constructor(&self.source[intrinsic_start..end])
                {
                    self.pos = end;
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
                self.pos = tag_end;
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
                let (string, end) =
                    crate::strings::scan(self.source, self.pos, self.limits.max_scalar_bytes)?;
                self.pos = end;
                Value::String(string)
            }
            Some('"') => {
                let (string, end) =
                    crate::strings::scan(self.source, self.pos, self.limits.max_scalar_bytes)?;
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
                Value::Bool(true)
            }
            Some('f') if self.keyword("false") => {
                self.pos += 5;
                Value::Bool(false)
            }
            Some('n') if self.keyword("null") => {
                self.pos += 4;
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
        Ok(Node {
            span: Span {
                start,
                end: self.pos,
            },
            tag,
            value,
        })
    }

    fn temporal_literal(
        &mut self,
        start: usize,
        tag: Option<Span>,
        kind: crate::TemporalKind,
    ) -> Result<Node, ParseError> {
        self.count_node(start)?;
        let payload_start = self.pos;
        if !matches!(self.peek(), Some('"' | '\'')) {
            return Err(self.error(ErrorKind::InvalidTemporal, start, self.char_end(self.pos)));
        }
        let (text, end) = crate::strings::scan(self.source, self.pos, self.limits.max_scalar_bytes)
            .map_err(|error| {
                if error.kind == ErrorKind::LimitExceeded {
                    error
                } else {
                    self.error(ErrorKind::InvalidTemporal, payload_start, error.span.end)
                }
            })?;
        self.pos = end;
        let value = crate::ParsedTemporal::parse(kind, text, self.source)
            .map_err(|_| self.error(ErrorKind::InvalidTemporal, payload_start, end))?;
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
        self.pos += 1;
        let mut entries = Vec::new();
        let mut keys = HashSet::new();
        let mut separated = true;
        loop {
            let newline = self.trivia()?;
            if self.peek() == Some('}') {
                self.pos += 1;
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
            if !valid_key(&self.source[key_start..self.pos]) {
                return Err(self.error(ErrorKind::ExpectedKey, key_start, self.pos));
            }
            let key = Span {
                start: key_start,
                end: self.pos,
            };
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
            self.pos += 1;
            let value = self.value(depth)?;
            entries.push(Entry { key, value });
            separated = false;
        }
    }

    fn sequence(&mut self, depth: usize) -> Result<Value, ParseError> {
        let open = self.pos;
        if depth > self.limits.max_depth || depth > HARD_MAX_DEPTH {
            return Err(self.error(ErrorKind::LimitExceeded, open, self.char_end(open)));
        }
        self.pos += 1;
        let mut items = Vec::new();
        let mut separated = true;
        loop {
            let newline = self.trivia()?;
            if self.peek() == Some(']') {
                self.pos += 1;
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
