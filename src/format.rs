use crate::{Document, Entry, Node, Span, Value, parser};
use pretty::{Arena, DocAllocator, DocBuilder};
use std::{error::Error, fmt};

/// Structural formatter options. String contents are never word-reflowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// Target display width. Unbreakable tokens and comments may exceed it.
    ///
    /// Any positive width is supported. The default is 80 columns.
    pub line_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self { line_width: 80 }
    }
}

impl FormatOptions {
    /// Check options before reading input or constructing layout documents.
    ///
    /// # Errors
    ///
    /// Returns [`FormatError::InvalidLineWidth`] for a zero width.
    pub fn validate(self) -> Result<(), FormatError> {
        if self.line_width == 0 {
            Err(FormatError::InvalidLineWidth)
        } else {
            Ok(())
        }
    }
}

/// A formatter option error. Input syntax is checked by [`crate::parse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// The requested line width is zero.
    InvalidLineWidth,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineWidth => f.write_str("line width must be positive"),
        }
    }
}

impl Error for FormatError {}

/// Reflow a parsed document while retaining comments, value spelling and tags.
///
/// Indentation is two spaces. Multiple collection entries remain newline-separated;
/// single-entry containers expand or collapse to the requested width. Block string
/// bodies are preserved apart from physical line-ending normalization; their closing
/// delimiter is aligned with its surrounding structural indentation. Output ends in
/// one LF. A leading BOM is retained. No schema resolution or defaults are applied.
///
/// # Errors
///
/// Returns [`FormatError::InvalidLineWidth`] for a zero width.
///
/// ```
/// use daml::{FormatOptions, format, parse};
/// let document = parse("{ task:{ enabled:true } }")?;
/// let formatted = format(&document, FormatOptions { line_width: 18 })?;
/// assert_eq!(formatted, "{\n  task: {\n    enabled: true\n  }\n}\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn format(document: &Document, options: FormatOptions) -> Result<String, FormatError> {
    options.validate()?;
    let arena = Arena::new();
    let printer = Printer {
        source: document.source(),
        arena: &arena,
    };
    let bom_end = usize::from(printer.source.starts_with('\u{feff}')) * 3;
    let (leading, has_leading) = printer.comments(bom_end, document.root().span.start);
    let mut output_doc = arena.text(&printer.source[..bom_end]).append(leading);
    if has_leading {
        output_doc = output_doc.append(arena.hardline());
    }
    output_doc = output_doc.append(printer.node(document.root()));
    let (trailing, _) = printer.comments(document.root().span.end, printer.source.len());
    output_doc = output_doc.append(trailing).append(arena.hardline());
    let mut output = String::with_capacity(document.source().len());
    output_doc
        .render_fmt(options.line_width, &mut output)
        .expect("rendering into a String cannot fail");
    Ok(output)
}

type Printed<'a> = DocBuilder<'a, Arena<'a>>;

struct Printer<'a> {
    source: &'a str,
    arena: &'a Arena<'a>,
}

impl<'a> Printer<'a> {
    fn node(&self, node: &Node) -> Printed<'a> {
        let mut start = node.tag.map_or(node.span.start, |tag| tag.end);
        while matches!(self.source.as_bytes().get(start), Some(b' ' | b'\t'))
            && start < node.span.end
        {
            start += 1;
        }
        if start == node.span.end {
            return self
                .arena
                .text(&self.source[node.span.start..node.span.end]);
        }
        let payload = match &node.value {
            Value::Mapping(entries) => self.collection(
                ("{", "}"),
                Span {
                    start,
                    end: node.span.end,
                },
                entries.iter().map(|entry| {
                    (
                        Span {
                            start: entry.key.start,
                            end: entry.value.span.end,
                        },
                        self.entry(entry),
                    )
                }),
                self.arena.line(),
            ),
            Value::Sequence(items) => self.collection(
                ("[", "]"),
                Span {
                    start,
                    end: node.span.end,
                },
                items.iter().map(|item| (item.span, self.node(item))),
                self.arena.line_(),
            ),
            Value::String(_) => {
                let raw = &self.source[start..node.span.end];
                if raw.starts_with("\"\"\"") || raw.starts_with("'''") {
                    self.block_string(raw)
                } else {
                    self.arena.text(raw)
                }
            }
            _ => self.arena.text(&self.source[start..node.span.end]),
        };
        if let Some(tag) = node.tag {
            self.arena
                .text(&self.source[node.span.start..tag.end])
                .append(self.arena.space())
                .append(payload)
        } else {
            payload
        }
    }

    fn entry(&self, entry: &Entry) -> Printed<'a> {
        let colon = entry.key.end
            + self.source[entry.key.end..entry.value.span.start]
                .find(':')
                .expect("a parsed mapping entry contains its colon");
        let head = self
            .arena
            .text(&self.source[entry.key.start..entry.key.end])
            .append(":");
        let value = self.node(&entry.value);
        let (comments, has_comments) = self.comments(colon + 1, entry.value.span.start);
        if has_comments {
            head.append(comments.append(self.arena.hardline()).append(value).nest(2))
        } else if matches!(entry.value.value, Value::Mapping(_) | Value::Sequence(_))
            || self.source[entry.value.span.start..entry.value.span.end].contains(['\r', '\n'])
        {
            head.append(self.arena.space()).append(value)
        } else {
            head.append(self.arena.line().append(value).nest(2)).group()
        }
    }

    fn collection(
        &self,
        delimiters: (&'static str, &'static str),
        span: Span,
        items: impl ExactSizeIterator<Item = (Span, Printed<'a>)>,
        border: Printed<'a>,
    ) -> Printed<'a> {
        let count = items.len();
        let border = if count > 1 {
            self.arena.hardline()
        } else {
            border
        };
        let mut inside = self.arena.nil();
        let mut end = span.start + 1;
        for (index, (item_span, item)) in items.enumerate() {
            let (comments, has_comments) = self.comments(end, item_span.start);
            inside = inside.append(comments);
            inside = inside.append(if index == 0 && !has_comments {
                border.clone()
            } else {
                self.arena.hardline()
            });
            inside = inside.append(item);
            end = item_span.end;
        }
        let (comments, has_comments) = self.comments(end, span.end - 1);
        inside = inside.append(comments);
        if count == 0 && !has_comments {
            return self.arena.text(delimiters.0).append(delimiters.1);
        }
        let closing = if count == 0 || has_comments {
            self.arena.hardline()
        } else {
            border
        };
        self.arena
            .text(delimiters.0)
            .append(inside.nest(2))
            .append(closing)
            .append(delimiters.1)
            .group()
    }

    // Comments share the parser's scanner; no second lexer or AST parse is needed.
    // The result has no trailing separator, so its caller controls closing indentation.
    fn comments(&self, start: usize, end: usize) -> (Printed<'a>, bool) {
        let mut printed = self.arena.nil();
        let mut cursor = start;
        let mut has_comments = false;
        let at_start = start == 0 || (start == 3 && self.source.starts_with('\u{feff}'));
        let (scanned, _) = parser::scan_trivia(self.source, start, end, |span| {
            let prefix = if !has_comments && at_start {
                self.arena.nil()
            } else if self.source[cursor..span.start].contains(['\r', '\n']) {
                self.arena.hardline()
            } else {
                self.arena.space()
            };
            printed = std::mem::replace(&mut printed, self.arena.nil())
                .append(prefix)
                .append(self.arena.text(&self.source[span.start..span.end]));
            cursor = span.end;
            has_comments = true;
        })
        .expect("a parsed document contains valid trivia");
        assert_eq!(scanned, end, "formatting gaps contain only parsed trivia");
        (printed, has_comments)
    }

    fn block_string(&self, raw: &'a str) -> Printed<'a> {
        let opening_end = raw.find(['\r', '\n']).expect("block opener has a newline");
        let body_start = after_line_end(raw, opening_end);
        let closing_break = raw
            .rfind(['\r', '\n'])
            .expect("block close has a preceding newline");
        let body_end = if raw.as_bytes()[closing_break] == b'\n'
            && closing_break > 0
            && raw.as_bytes()[closing_break - 1] == b'\r'
        {
            closing_break - 1
        } else {
            closing_break
        };
        let opening = self.arena.text(&raw[..opening_end]);
        let closing = self.arena.text(&raw[raw.len() - 3..]);
        if body_start >= body_end {
            return opening.append(self.arena.hardline()).append(closing);
        }
        let body = self.literal_lines(&raw[body_start..body_end]).into_doc();
        let arena = self.arena;
        // Literal blank-only bodies have zero dedent margin. Indenting every body
        // line would change their value; cancel structural indentation for the body.
        let literal = arena.nesting(move |indent| {
            // `pretty` chooses a hardline's indentation from the next document.
            // Keep a zero-width command at the body indentation even when the
            // preserved body is empty, or structural indentation becomes data.
            let boundary = arena.column(|_| arena.nil().into_doc());
            arena
                .hardline()
                .append(body)
                .append(boundary)
                .nest(-(indent as isize))
                .into_doc()
        });
        opening
            .append(literal)
            .append(arena.hardline())
            .append(closing)
    }

    fn literal_lines(&self, raw: &'a str) -> Printed<'a> {
        let mut printed = self.arena.nil();
        let mut start = 0;
        while let Some(relative) = raw[start..].find(['\r', '\n']) {
            let end = start + relative;
            printed = printed
                .append(self.arena.text(&raw[start..end]))
                .append(self.arena.hardline());
            start = after_line_end(raw, end);
        }
        printed.append(self.arena.text(&raw[start..]))
    }
}

fn after_line_end(source: &str, at: usize) -> usize {
    at + if source.as_bytes()[at] == b'\r' && source.as_bytes().get(at + 1) == Some(&b'\n') {
        2
    } else {
        1
    }
}
