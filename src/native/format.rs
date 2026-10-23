use crate::format::{Printed, Printer};
use crate::native::{DefaultValue, Definition, Expression, ExpressionKind, Member};
use crate::{FormatError, FormatOptions, Span, Value};
use pretty::{Arena, DocAllocator};

/// Formats a parsed native definition or ordinary DAML data expression.
///
/// Structural entries are reflowed to `options.line_width`; comments, scalar
/// spelling, and block-string contents are retained.
///
/// # Errors
///
/// Returns [`FormatError::InvalidLineWidth`] when `line_width` is zero.
pub fn format_definition(
    definition: &Definition,
    options: FormatOptions,
) -> Result<String, FormatError> {
    options.validate()?;
    let arena = Arena::new();
    let printer = Printer {
        source: definition.source(),
        arena: &arena,
    };
    Ok(printer.finish(
        definition.span(),
        render(&printer, &definition.root, 0),
        options,
    ))
}

fn render<'a>(printer: &Printer<'a>, expression: &Expression, parent: u8) -> Printed<'a> {
    let arena = printer.arena;
    let (mut doc, precedence) = match &expression.kind {
        ExpressionKind::Literal(node) => (printer.node(node), 3),
        ExpressionKind::Regex(pattern) => (
            arena
                .text("!Regex \"")
                .append(arena.text(&printer.source[pattern.start..pattern.end]))
                .append("\""),
            3,
        ),
        ExpressionKind::Tagged { name, body } => {
            let tag = arena
                .text("!")
                .append(arena.text(&printer.source[name.start..name.end]));
            let value = match body {
                Some(body) => tag.append(arena.space()).append(render(printer, body, 0)),
                None => tag,
            };
            (value, 3)
        }
        ExpressionKind::Record(members) => {
            let items = members
                .iter()
                .map(|member| (member.span, render_member(printer, member)));
            (
                printer.collection(
                    ("{", "}"),
                    collection_span(printer, expression),
                    items,
                    arena.line(),
                ),
                3,
            )
        }
        ExpressionKind::Tuple(items) => {
            let docs = items
                .iter()
                .map(|item| (item.span, render(printer, item, 0)));
            (
                printer.collection(
                    ("[", "]"),
                    collection_span(printer, expression),
                    docs,
                    arena.line_(),
                ),
                3,
            )
        }
        ExpressionKind::Group(inner) => (
            printer.collection(
                ("(", ")"),
                collection_span(printer, expression),
                std::iter::once((inner.span, render(printer, inner, 0))),
                arena.line_(),
            ),
            3,
        ),
        ExpressionKind::Union(items) => (render_joined(printer, items, '|'), 1),
        ExpressionKind::Intersection(items) => (render_joined(printer, items, '&'), 2),
        ExpressionKind::Closed(inner) => {
            let (comments, has_comments) =
                printer.comments(expression.span.start + "closed".len(), inner.span.start);
            let separator = if has_comments {
                comments.append(arena.hardline())
            } else {
                arena.space()
            };
            (
                arena
                    .text("closed")
                    .append(separator)
                    .append(render(printer, inner, 3)),
                3,
            )
        }
    };

    if precedence < parent {
        doc = arena.text("(").append(doc).append(")");
    }
    if !matches!(&expression.default, DefaultValue::Inherited) {
        doc = render_default(
            printer,
            doc,
            &expression.default,
            expression.assignment,
            core_end(printer, expression),
            expression.span.end,
        );
    }
    doc
}

fn render_joined<'a>(printer: &Printer<'a>, items: &[Expression], operator: char) -> Printed<'a> {
    let arena = printer.arena;
    let mut docs = items.iter();
    let Some(first) = docs.next() else {
        return arena.nil();
    };
    let mut result = render(printer, first, if operator == '|' { 2 } else { 3 });
    let mut previous = first;
    for item in docs {
        let operator_at =
            crate::parser::scan_trivia(printer.source, previous.span.end, item.span.start, |_| {})
                .map_or(previous.span.end, |(end, _)| end);
        let (before, had_before) = printer.comments(previous.span.end, operator_at);
        let (after, had_after) =
            printer.comments(operator_at + operator.len_utf8(), item.span.start);
        result = result
            .append(before)
            .append(if had_before {
                arena.hardline()
            } else {
                arena.line()
            })
            .append(arena.text(if operator == '|' { "|" } else { "&" }));
        if had_after {
            result = result.append(after).append(arena.hardline());
        } else {
            result = result.append(arena.line());
        }
        result = result.append(render(printer, item, if operator == '|' { 2 } else { 3 }));
        previous = item;
    }
    result.group()
}

fn render_member<'a>(printer: &Printer<'a>, member: &Member) -> Printed<'a> {
    let arena = printer.arena;
    let mut head = arena.text(&printer.source[member.key.start..member.key.end]);
    if member.optional {
        head = head.append("?");
    }
    let between = &printer.source[member.key.end..member.constraint.span.start];
    let colon_at = between
        .find(':')
        .map_or(member.key.end, |index| member.key.end + index);
    let (comments, has_comments) = printer.comments(colon_at + 1, member.constraint.span.start);
    let value = render(printer, &member.constraint, 0);
    if has_comments {
        head.append(":")
            .append(comments.append(arena.hardline()).append(value).nest(2))
    } else if begins_with_collection(&member.constraint) {
        head.append(":").append(arena.space()).append(value)
    } else {
        head.append(":")
            .append(arena.line().append(value).nest(2))
            .group()
    }
}

fn begins_with_collection(expression: &Expression) -> bool {
    match &expression.kind {
        ExpressionKind::Record(_) | ExpressionKind::Tuple(_) => true,
        ExpressionKind::Literal(node) => {
            matches!(&node.value, Value::Mapping(_) | Value::Sequence(_))
        }
        ExpressionKind::Tagged { body, .. } => body.as_deref().is_some_and(begins_with_collection),
        ExpressionKind::Group(inner) | ExpressionKind::Closed(inner) => {
            begins_with_collection(inner)
        }
        ExpressionKind::Regex(_) | ExpressionKind::Union(_) | ExpressionKind::Intersection(_) => {
            false
        }
    }
}

fn collection_span(printer: &Printer<'_>, expression: &Expression) -> Span {
    Span {
        start: expression.span.start,
        end: core_end(printer, expression),
    }
}

fn core_end(printer: &Printer<'_>, expression: &Expression) -> usize {
    let source = printer.source;
    match &expression.kind {
        ExpressionKind::Record(members) => {
            if expression.assignment.is_none() {
                return expression.span.end;
            }
            let from = members
                .last()
                .map_or(expression.span.start + 1, |member| member.span.end);
            let close = crate::parser::scan_trivia(
                source,
                from,
                expression.assignment.unwrap().start,
                |_| {},
            )
            .map_or(from, |(end, _)| end);
            close + 1
        }
        ExpressionKind::Tuple(items) => {
            if expression.assignment.is_none() {
                return expression.span.end;
            }
            let from = items
                .last()
                .map_or(expression.span.start + 1, |item| item.span.end);
            let close = crate::parser::scan_trivia(
                source,
                from,
                expression.assignment.unwrap().start,
                |_| {},
            )
            .map_or(from, |(end, _)| end);
            close + 1
        }
        ExpressionKind::Group(inner) => {
            let limit = expression
                .assignment
                .map_or(expression.span.end, |assignment| assignment.start);
            let close = crate::parser::scan_trivia(source, inner.span.end, limit, |_| {})
                .map_or(inner.span.end, |(end, _)| end);
            close + 1
        }
        ExpressionKind::Literal(node) => node.span.end,
        ExpressionKind::Regex(pattern) => pattern.end + 1,
        ExpressionKind::Tagged { name, body } => body
            .as_ref()
            .map_or(name.end, |body| core_end(printer, body)),
        ExpressionKind::Union(items) | ExpressionKind::Intersection(items) => items
            .last()
            .map_or(expression.span.start, |item| core_end(printer, item)),
        ExpressionKind::Closed(inner) => core_end(printer, inner),
    }
}

fn render_default<'a>(
    printer: &Printer<'a>,
    body: Printed<'a>,
    default: &DefaultValue,
    assignment: Option<Span>,
    base_end: usize,
    source_end: usize,
) -> Printed<'a> {
    let arena = printer.arena;
    let separator = assignment.unwrap_or(Span {
        start: base_end,
        end: base_end,
    });
    let (rhs_start, rhs) = match default {
        DefaultValue::Inherited => return body,
        DefaultValue::Unset => {
            let start =
                crate::parser::scan_trivia(printer.source, separator.end, source_end, |_| {})
                    .map_or(separator.end, |(end, _)| end);
            (start, arena.text("unset"))
        }
        DefaultValue::Value(node) => (node.span.start, printer.node(node)),
    };
    let (before, has_before) = printer.comments(base_end, separator.start);
    let body = body
        .append(before)
        .append(if has_before {
            arena.hardline()
        } else {
            arena.line()
        })
        .append("=");
    let (after, has_after) = printer.comments(separator.end, rhs_start);
    if has_after {
        body.append(after).append(arena.hardline()).append(rhs)
    } else {
        body.append(arena.line()).append(rhs)
    }
    .group()
}
