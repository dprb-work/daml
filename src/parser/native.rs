use super::{Parser, check_input, is_name_char};
use crate::names::valid_key;
use crate::native::{DefaultValue, Expression, ExpressionKind, Member};
use crate::{ErrorKind, Limits, ParseError, Span};

pub(crate) fn parse_definition(source: &str, limits: Limits) -> Result<Expression, ParseError> {
    check_input(source, limits)?;
    let mut parser = Parser {
        source,
        pos: usize::from(source.starts_with('\u{feff}')) * 3,
        limits,
        nodes: 0,
    };
    parser.trivia()?;
    if parser.pos == source.len() {
        return Err(parser.error(ErrorKind::ExpectedValue, parser.pos, parser.pos));
    }
    let root = parser.expression(0, false)?;
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

impl Parser<'_> {
    fn expression(&mut self, depth: usize, tuple_position: bool) -> Result<Expression, ParseError> {
        self.trivia()?;
        let start = self.pos;
        if depth > self.limits.max_depth || depth > 128 {
            return Err(self.error(ErrorKind::LimitExceeded, start, self.char_end(start)));
        }
        self.count_node(start)?;
        let mut left = self.intersection(depth, tuple_position)?;
        let start = left.span.start;
        let mut alternatives = vec![left];
        loop {
            let save = self.pos;
            self.trivia()?;
            if self.peek() != Some('|') {
                self.pos = save;
                break;
            }
            self.pos += 1;
            self.trivia()?;
            alternatives.push(self.intersection(depth, tuple_position)?);
        }
        left = if alternatives.len() == 1 {
            alternatives.pop().unwrap()
        } else {
            let end = alternatives.last().unwrap().span.end;
            Expression {
                span: Span { start, end },
                kind: ExpressionKind::Union(alternatives),
                default: DefaultValue::Inherited,
                assignment: None,
            }
        };
        let before_default = self.pos;
        self.trivia()?;
        if self.peek() == Some('=') {
            if tuple_position {
                return Err(self.error(
                    ErrorKind::UnexpectedToken,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            let assignment = self.pos;
            self.pos += 1;
            self.trivia()?;
            left.default = if self.source[self.pos..].starts_with("unset") {
                let end = self.pos + 5;
                if self.source[end..]
                    .chars()
                    .next()
                    .is_none_or(|ch| !is_name_char(ch))
                {
                    self.pos = end;
                    DefaultValue::Unset
                } else {
                    DefaultValue::Value(self.value(depth)?)
                }
            } else {
                DefaultValue::Value(self.value(depth)?)
            };
            left.span.end = self.pos;
            left.assignment = Some(Span {
                start: assignment,
                end: assignment + 1,
            });
        } else {
            self.pos = before_default;
        }
        Ok(left)
    }

    fn intersection(
        &mut self,
        depth: usize,
        tuple_position: bool,
    ) -> Result<Expression, ParseError> {
        let mut values = vec![self.atom(depth, tuple_position)?];
        loop {
            let save = self.pos;
            self.trivia()?;
            if self.peek() != Some('&') {
                self.pos = save;
                break;
            }
            self.pos += 1;
            self.trivia()?;
            values.push(self.atom(depth, tuple_position)?);
        }
        if values.len() == 1 {
            return Ok(values.pop().unwrap());
        }
        let start = values[0].span.start;
        let end = values.last().unwrap().span.end;
        Ok(Expression {
            span: Span { start, end },
            kind: ExpressionKind::Intersection(values),
            default: DefaultValue::Inherited,
            assignment: None,
        })
    }

    fn atom(&mut self, depth: usize, tuple_position: bool) -> Result<Expression, ParseError> {
        self.trivia()?;
        if depth > self.limits.max_depth || depth > 128 {
            return Err(self.error(ErrorKind::LimitExceeded, self.pos, self.char_end(self.pos)));
        }
        self.count_node(self.pos)?;
        let start = self.pos;
        if self.peek() == Some('(') {
            self.pos += 1;
            self.trivia()?;
            let inner = self.expression(depth + 1, tuple_position)?;
            self.trivia()?;
            if self.peek() != Some(')') {
                return Err(self.error(
                    ErrorKind::UnexpectedToken,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            self.pos += 1;
            return Ok(Expression {
                span: Span {
                    start,
                    end: self.pos,
                },
                kind: ExpressionKind::Group(Box::new(inner)),
                default: DefaultValue::Inherited,
                assignment: None,
            });
        }
        if self.source[self.pos..].starts_with("closed") {
            self.pos += 6;
            self.horizontal();
            let inner = Box::new(self.atom(depth + 1, tuple_position)?);
            return Ok(Expression {
                span: Span {
                    start,
                    end: self.pos,
                },
                kind: ExpressionKind::Closed(inner),
                default: DefaultValue::Inherited,
                assignment: None,
            });
        }
        if self.peek() == Some('!') {
            self.pos += 1;
            let name_start = self.pos;
            while self.peek().is_some_and(is_name_char) {
                self.pos += self.peek().unwrap().len_utf8();
            }
            if self.pos == name_start {
                return Err(self.error(ErrorKind::InvalidTag, name_start, self.pos));
            }
            if !crate::names::valid_tag(&self.source[name_start..self.pos]) {
                return Err(self.error(ErrorKind::InvalidTag, name_start, self.pos));
            }
            if self.pos - name_start > self.limits.max_scalar_bytes {
                return Err(self.error(ErrorKind::LimitExceeded, name_start, self.pos));
            }
            let name = Span {
                start: name_start,
                end: self.pos,
            };
            if self.source.get(name.start..name.end) == Some("Regex") {
                self.horizontal();
                return self.regex(start);
            }
            let save = self.pos;
            self.horizontal();
            if self.peek() == Some('!') {
                return Err(self.error(ErrorKind::InvalidTag, self.pos, self.char_end(self.pos)));
            }
            if self.source.get(name.start..name.end) == Some("Tuple") {
                if self.peek() == Some('[') {
                    let body = Box::new(self.tuple(depth + 1, true)?);
                    return Ok(Expression {
                        span: Span {
                            start,
                            end: body.span.end,
                        },
                        kind: ExpressionKind::Tagged {
                            name,
                            body: Some(body),
                        },
                        default: DefaultValue::Inherited,
                        assignment: None,
                    });
                }
                if self.peek() == Some('{') {
                    let body = Box::new(self.record(depth + 1, false)?);
                    return Ok(Expression {
                        span: Span {
                            start,
                            end: body.span.end,
                        },
                        kind: ExpressionKind::Tagged {
                            name,
                            body: Some(body),
                        },
                        default: DefaultValue::Inherited,
                        assignment: None,
                    });
                }
            }
            if self.peek() == Some('{') {
                let metadata =
                    matches!(self.source.get(name.start..name.end), Some("List" | "Map"));
                let body = Box::new(self.record(depth + 1, metadata)?);
                return Ok(Expression {
                    span: Span {
                        start,
                        end: body.span.end,
                    },
                    kind: ExpressionKind::Tagged {
                        name,
                        body: Some(body),
                    },
                    default: DefaultValue::Inherited,
                    assignment: None,
                });
            }
            if self.peek() == Some('[') {
                let node = self.value(depth + 1)?;
                let end = node.span.end;
                let body = Box::new(Expression {
                    span: node.span,
                    kind: ExpressionKind::Literal(node),
                    default: DefaultValue::Inherited,
                    assignment: None,
                });
                return Ok(Expression {
                    span: Span { start, end },
                    kind: ExpressionKind::Tagged {
                        name,
                        body: Some(body),
                    },
                    default: DefaultValue::Inherited,
                    assignment: None,
                });
            }
            if self.peek().is_some_and(|ch| {
                matches!(ch, '"' | '\'' | 't' | 'f' | 'n' | '-' | '.' | '0'..='9')
            }) {
                let node = self.value(depth + 1)?;
                let end = node.span.end;
                let body = Box::new(Expression {
                    span: node.span,
                    kind: ExpressionKind::Literal(node),
                    default: DefaultValue::Inherited,
                    assignment: None,
                });
                return Ok(Expression {
                    span: Span { start, end },
                    kind: ExpressionKind::Tagged {
                        name,
                        body: Some(body),
                    },
                    default: DefaultValue::Inherited,
                    assignment: None,
                });
            }
            self.pos = save;
            return Ok(Expression {
                span: Span {
                    start,
                    end: self.pos,
                },
                kind: ExpressionKind::Tagged { name, body: None },
                default: DefaultValue::Inherited,
                assignment: None,
            });
        }
        if self.peek() == Some('{') {
            return self.record(depth, false);
        }
        let node = self.value(depth)?;
        let end = node.span.end;
        Ok(Expression {
            span: Span { start, end },
            kind: ExpressionKind::Literal(node),
            default: DefaultValue::Inherited,
            assignment: None,
        })
    }

    fn tuple(&mut self, depth: usize, tuple_position: bool) -> Result<Expression, ParseError> {
        let start = self.pos;
        self.count_node(start)?;
        if depth > self.limits.max_depth || depth > 128 {
            return Err(self.error(ErrorKind::LimitExceeded, self.pos, self.char_end(self.pos)));
        }
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            let newline = self.trivia()?;
            if self.peek() == Some(']') {
                self.pos += 1;
                break;
            }
            if self.pos == self.source.len() {
                return Err(self.error(ErrorKind::UnterminatedCollection, start, self.pos));
            }
            if !items.is_empty() && !newline {
                return Err(self.error(
                    ErrorKind::ExpectedSeparator,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            items.push(self.expression(depth + 1, tuple_position)?);
        }
        Ok(Expression {
            span: Span {
                start,
                end: self.pos,
            },
            kind: ExpressionKind::Tuple(items),
            default: DefaultValue::Inherited,
            assignment: None,
        })
    }

    fn regex(&mut self, start: usize) -> Result<Expression, ParseError> {
        if self.peek() != Some('"') {
            return Err(self.error(ErrorKind::ExpectedValue, self.pos, self.char_end(self.pos)));
        }
        self.pos += 1;
        let pattern_start = self.pos;
        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error(ErrorKind::UnterminatedString, start, self.pos));
            };
            match ch {
                '"' => {
                    let pattern = Span {
                        start: pattern_start,
                        end: self.pos,
                    };
                    if pattern.end - pattern.start > self.limits.max_scalar_bytes {
                        return Err(self.error(
                            ErrorKind::LimitExceeded,
                            pattern.start,
                            pattern.end,
                        ));
                    }
                    self.pos += 1;
                    return Ok(Expression {
                        span: Span {
                            start,
                            end: self.pos,
                        },
                        kind: ExpressionKind::Regex(pattern),
                        default: DefaultValue::Inherited,
                        assignment: None,
                    });
                }
                '\r' | '\n' => {
                    return Err(self.error(
                        ErrorKind::InvalidString,
                        self.pos,
                        self.char_end(self.pos),
                    ));
                }
                '\u{feff}' if self.pos != 0 => {
                    return Err(self.error(
                        ErrorKind::InvalidCharacter,
                        self.pos,
                        self.char_end(self.pos),
                    ));
                }
                _ if ch.is_ascii_control() => {
                    return Err(self.error(
                        ErrorKind::InvalidCharacter,
                        self.pos,
                        self.char_end(self.pos),
                    ));
                }
                _ => self.pos += ch.len_utf8(),
            }
        }
    }

    fn record(&mut self, depth: usize, metadata: bool) -> Result<Expression, ParseError> {
        let start = self.pos;
        if depth > self.limits.max_depth || depth > 128 {
            return Err(self.error(ErrorKind::LimitExceeded, self.pos, self.char_end(self.pos)));
        }
        self.pos += 1;
        let mut members = Vec::new();
        let mut keys = std::collections::HashSet::new();
        let mut separated = true;
        loop {
            let newline = self.trivia()?;
            if self.peek() == Some('}') {
                self.pos += 1;
                break;
            }
            if self.pos == self.source.len() {
                return Err(self.error(ErrorKind::UnterminatedCollection, start, self.pos));
            }
            if !separated && !newline {
                return Err(self.error(
                    ErrorKind::ExpectedSeparator,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            let key_start = self.pos;
            self.count_node(key_start)?;
            while self.peek().is_some_and(is_key_char) {
                self.pos += self.peek().unwrap().len_utf8();
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
            if !keys.insert(&self.source[key.start..key.end]) {
                return Err(self.error(ErrorKind::DuplicateKey, key.start, key.end));
            }
            if key_start == self.pos {
                return Err(self.error(ErrorKind::ExpectedKey, self.pos, self.char_end(self.pos)));
            }
            let optional = self.peek() == Some('?');
            if optional {
                self.pos += 1;
            }
            self.horizontal();
            if self.peek() != Some(':') {
                if self.peek() == Some('=') {
                    return Err(self.error(
                        ErrorKind::UnexpectedToken,
                        self.pos,
                        self.char_end(self.pos),
                    ));
                }
                return Err(self.error(
                    ErrorKind::ExpectedColon,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            self.pos += 1;
            self.horizontal();
            let mut constraint = self.expression(depth + 1, false)?;
            let before_default = self.pos;
            self.trivia()?;
            if self.peek() == Some('=')
                && !matches!(constraint.effective_default(), DefaultValue::Inherited)
            {
                return Err(self.error(
                    ErrorKind::UnexpectedToken,
                    self.pos,
                    self.char_end(self.pos),
                ));
            }
            if self.peek() == Some('=') {
                let assignment = self.pos;
                self.pos += 1;
                self.trivia()?;
                constraint.default = if self.source[self.pos..].starts_with("unset")
                    && self.source[self.pos + 5..]
                        .chars()
                        .next()
                        .is_none_or(|ch| !is_name_char(ch))
                {
                    self.pos += 5;
                    DefaultValue::Unset
                } else {
                    DefaultValue::Value(self.value(depth + 1)?)
                };
                constraint.span.end = self.pos;
                constraint.assignment = Some(Span {
                    start: assignment,
                    end: assignment + 1,
                });
            } else {
                self.pos = before_default;
            }
            if !metadata && let Some(assignment) = first_scalar_default_assignment(&constraint) {
                return Err(self.error(
                    ErrorKind::UnexpectedToken,
                    assignment.start,
                    assignment.end,
                ));
            }
            if optional && !matches!(constraint.effective_default(), DefaultValue::Inherited) {
                return Err(self.error(ErrorKind::UnexpectedToken, key.start, self.pos));
            }
            members.push(Member {
                span: Span {
                    start: key.start,
                    end: constraint.span.end,
                },
                key,
                optional,
                constraint,
            });
            separated = false;
        }
        Ok(Expression {
            span: Span {
                start,
                end: self.pos,
            },
            kind: ExpressionKind::Record(members),
            default: DefaultValue::Inherited,
            assignment: None,
        })
    }
}

fn first_scalar_default_assignment(expression: &Expression) -> Option<Span> {
    if matches!(
        &expression.default,
        DefaultValue::Value(node)
            if !matches!(&node.value, crate::Value::Mapping(_) | crate::Value::Sequence(_))
    ) {
        return expression.assignment;
    }
    match &expression.kind {
        ExpressionKind::Group(inner) | ExpressionKind::Closed(inner) => {
            first_scalar_default_assignment(inner)
        }
        ExpressionKind::Union(items) | ExpressionKind::Intersection(items) => {
            items.iter().find_map(first_scalar_default_assignment)
        }
        ExpressionKind::Literal(_)
        | ExpressionKind::Regex(_)
        | ExpressionKind::Tagged { .. }
        | ExpressionKind::Record(_)
        | ExpressionKind::Tuple(_) => None,
    }
}

fn is_key_char(ch: char) -> bool {
    !ch.is_whitespace()
        && !matches!(
            ch,
            '{' | '}'
                | '['
                | ']'
                | ':'
                | '?'
                | '='
                | '#'
                | ','
                | '"'
                | '\''
                | '!'
                | '&'
                | '|'
                | '>'
                | '%'
                | '\\'
        )
}
