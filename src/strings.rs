use crate::source::{line_end, newline_start};
use crate::{ErrorKind, ParseError, Span, StringValue};

/// Scans one supported quoted string beginning at `start`.
pub(crate) fn scan(
    source: &str,
    start: usize,
    max_bytes: usize,
) -> Result<(StringValue, usize), ParseError> {
    scan_inner(source, start, max_bytes).map_err(|mut error| {
        let mut start = error.span.start.min(source.len());
        while !source.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = error.span.end.min(source.len());
        while !source.is_char_boundary(end) {
            end += 1;
        }
        error.span = Span { start, end };
        error
    })
}

fn scan_inner(
    source: &str,
    start: usize,
    max_bytes: usize,
) -> Result<(StringValue, usize), ParseError> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'"') && bytes.get(start) != Some(&b'\'') {
        return Err(ParseError::new(ErrorKind::InvalidString, start, start));
    }
    if bytes.get(start..start.saturating_add(3)) == Some(b"\"\"\"") {
        scan_triple(source, start, max_bytes, b'"')
    } else if bytes.get(start..start.saturating_add(3)) == Some(b"'''") {
        scan_triple(source, start, max_bytes, b'\'')
    } else if bytes[start] == b'"' {
        scan_ordinary(source, start, max_bytes)
    } else {
        Err(ParseError::new(ErrorKind::InvalidString, start, start + 1))
    }
}

fn over_limit(start: usize, end: usize, max_bytes: usize) -> bool {
    end.saturating_sub(start) > max_bytes
}

fn scan_ordinary(
    source: &str,
    start: usize,
    max_bytes: usize,
) -> Result<(StringValue, usize), ParseError> {
    let bytes = source.as_bytes();
    let mut i = start + 1;
    let mut decoded: Option<String> = None;
    while i < bytes.len() {
        if over_limit(start, i + 1, max_bytes) {
            return Err(ParseError::new(ErrorKind::LimitExceeded, start, i + 1));
        }
        match bytes[i] {
            b'"' => {
                let end = i + 1;
                if over_limit(start, end, max_bytes) {
                    return Err(ParseError::new(ErrorKind::LimitExceeded, start, end));
                }
                return Ok((
                    decoded.map_or_else(
                        || {
                            StringValue::Source(Span {
                                start: start + 1,
                                end: i,
                            })
                        },
                        StringValue::Decoded,
                    ),
                    end,
                ));
            }
            0..=0x1f => return Err(ParseError::new(ErrorKind::InvalidString, i, i + 1)),
            b'\\' => {
                let output = decoded.get_or_insert_with(|| source[start + 1..i].to_owned());
                let escape_start = i;
                i += 1;
                let Some(&escaped) = bytes.get(i) else {
                    return Err(ParseError::new(ErrorKind::InvalidEscape, escape_start, i));
                };
                let ch = match escaped {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{0008}',
                    b'f' => '\u{000c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let (ch, next) = unicode_escape(bytes, i, source.len())?;
                        output.push(ch);
                        i = next;
                        let copy_start = i;
                        while i < bytes.len() && bytes[i] != b'\\' && bytes[i] != b'"' {
                            if over_limit(start, i + 1, max_bytes) {
                                return Err(ParseError::new(
                                    ErrorKind::LimitExceeded,
                                    start,
                                    i + 1,
                                ));
                            }
                            if bytes[i] <= 0x1f {
                                return Err(ParseError::new(ErrorKind::InvalidString, i, i + 1));
                            }
                            i += source[i..].chars().next().unwrap().len_utf8();
                        }
                        output.push_str(&source[copy_start..i]);
                        continue;
                    }
                    _ => {
                        return Err(ParseError::new(
                            ErrorKind::InvalidEscape,
                            escape_start,
                            i + 1,
                        ));
                    }
                };
                output.push(ch);
                i += 1;
                let copy_start = i;
                while i < bytes.len() && bytes[i] != b'\\' && bytes[i] != b'"' {
                    if over_limit(start, i + 1, max_bytes) {
                        return Err(ParseError::new(ErrorKind::LimitExceeded, start, i + 1));
                    }
                    if bytes[i] <= 0x1f {
                        return Err(ParseError::new(ErrorKind::InvalidString, i, i + 1));
                    }
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                output.push_str(&source[copy_start..i]);
            }
            _ => i += source[i..].chars().next().unwrap().len_utf8(),
        }
    }
    Err(ParseError::new(
        ErrorKind::UnterminatedString,
        start,
        source.len(),
    ))
}

fn hex4(bytes: &[u8], at: usize) -> Option<u16> {
    let mut value = 0u16;
    for &b in bytes.get(at..at + 4)? {
        value = value.checked_mul(16)?
            + match b {
                b'0'..=b'9' => u16::from(b - b'0'),
                b'a'..=b'f' => u16::from(b - b'a' + 10),
                b'A'..=b'F' => u16::from(b - b'A' + 10),
                _ => return None,
            };
    }
    Some(value)
}

fn unicode_escape(
    bytes: &[u8],
    u_at: usize,
    source_len: usize,
) -> Result<(char, usize), ParseError> {
    let first_start = u_at + 1;
    let first = hex4(bytes, first_start).ok_or_else(|| {
        ParseError::new(
            ErrorKind::InvalidEscape,
            u_at - 1,
            (first_start + 4).min(source_len),
        )
    })?;
    let mut end = first_start + 4;
    let scalar = if (0xd800..=0xdbff).contains(&first) {
        if bytes.get(end..end + 2) != Some(b"\\u") {
            return Err(ParseError::new(
                ErrorKind::InvalidEscape,
                u_at - 1,
                end.min(source_len),
            ));
        }
        let second = hex4(bytes, end + 2).ok_or_else(|| {
            ParseError::new(
                ErrorKind::InvalidEscape,
                u_at - 1,
                (end + 6).min(source_len),
            )
        })?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err(ParseError::new(ErrorKind::InvalidEscape, u_at - 1, end + 6));
        }
        end += 6;
        0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
    } else if (0xdc00..=0xdfff).contains(&first) {
        return Err(ParseError::new(ErrorKind::InvalidEscape, u_at - 1, end));
    } else {
        u32::from(first)
    };
    Ok((
        char::from_u32(scalar).expect("validated Unicode scalar"),
        end,
    ))
}

fn scan_triple(
    source: &str,
    start: usize,
    max_bytes: usize,
    quote: u8,
) -> Result<(StringValue, usize), ParseError> {
    let bytes = source.as_bytes();
    let mut opener_end = start + 3;
    while matches!(bytes.get(opener_end), Some(b' ' | b'\t')) {
        if over_limit(start, opener_end + 1, max_bytes) {
            return Err(ParseError::new(
                ErrorKind::LimitExceeded,
                start,
                opener_end + 1,
            ));
        }
        opener_end += 1;
    }
    let Some(content_start) = line_end(bytes, opener_end) else {
        let end = bytes.get(opener_end).map_or(opener_end, |_| opener_end + 1);
        return Err(ParseError::new(ErrorKind::InvalidString, opener_end, end));
    };

    let delimiter = [quote; 3];
    let mut line_start = content_start;
    let mut close = None;
    while line_start <= bytes.len() {
        if over_limit(start, line_start, max_bytes) {
            return Err(ParseError::new(ErrorKind::LimitExceeded, start, line_start));
        }
        let mut p = line_start;
        while matches!(bytes.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        let mut q = line_start;
        while q + 2 < bytes.len() && !matches!(bytes[q], b'\r' | b'\n') {
            if over_limit(start, q + 1, max_bytes) {
                return Err(ParseError::new(ErrorKind::LimitExceeded, start, q + 1));
            }
            if bytes[q] < 0x20 && bytes[q] != b'\t' {
                return Err(ParseError::new(ErrorKind::InvalidString, q, q + 1));
            }
            if bytes.get(q..q + 3) == Some(delimiter.as_slice()) {
                let mut back = q;
                while back > line_start && bytes[back - 1] == b'\\' {
                    back -= 1;
                }
                if (q - back) % 2 == 1 {
                    q += 3;
                    continue;
                }
                if q != p {
                    return Err(ParseError::new(ErrorKind::InvalidString, q, q + 3));
                }
                let mut tail = q + 3;
                while matches!(bytes.get(tail), Some(b' ' | b'\t')) {
                    tail += 1;
                }
                let valid_tail = tail == bytes.len()
                    || line_end(bytes, tail).is_some()
                    || bytes.get(tail) == Some(&b'#');
                if valid_tail {
                    close = Some((line_start, q + 3));
                    break;
                }
                return Err(ParseError::new(ErrorKind::InvalidString, q, q + 3));
            }
            q += 1;
        }
        if close.is_some() || line_start == bytes.len() {
            break;
        }
        let mut eol = line_start;
        while eol < bytes.len() && !matches!(bytes[eol], b'\r' | b'\n') {
            eol += 1;
        }
        let Some(next) = line_end(bytes, eol) else {
            break;
        };
        line_start = next;
    }
    let Some((close_line, end)) = close else {
        return Err(ParseError::new(
            ErrorKind::UnterminatedString,
            start,
            source.len(),
        ));
    };
    if over_limit(start, end, max_bytes) {
        return Err(ParseError::new(ErrorKind::LimitExceeded, start, end));
    }
    let content_end = if close_line == content_start {
        close_line
    } else {
        newline_start(bytes, close_line)
    };
    let raw = &source[content_start..content_end];
    let value = if quote == b'\'' {
        wrap_lines(raw, content_start, quote)
    } else {
        dedent_and_join(raw, content_start, quote)?
    };
    Ok((value, end))
}

fn next_line(raw: &str, at: usize) -> (usize, usize) {
    let bytes = raw.as_bytes();
    let mut end = at;
    while end < bytes.len() && !matches!(bytes[end], b'\r' | b'\n') {
        end += 1;
    }
    let next = line_end(bytes, end).unwrap_or(end);
    (end, next)
}

fn append_triple_text(output: &mut String, text: &str, quote: u8) {
    let bytes = text.as_bytes();
    let delimiter = [quote; 3];
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            if bytes.get(at + 1) == Some(&b'\\') {
                output.push('\\');
                at += 2;
            } else if bytes.get(at + 1..at + 4) == Some(delimiter.as_slice()) {
                output.push_str(std::str::from_utf8(&delimiter).unwrap());
                at += 4;
            } else {
                output.push('\\');
                at += 1;
            }
        } else {
            let ch = text[at..].chars().next().unwrap();
            output.push(ch);
            at += ch.len_utf8();
        }
    }
}

fn triple_needs_decoding(text: &str, quote: u8) -> bool {
    let bytes = text.as_bytes();
    let delimiter = [quote; 3];
    bytes.iter().enumerate().any(|(at, byte)| {
        *byte == b'\\'
            && (bytes.get(at + 1) == Some(&b'\\')
                || bytes.get(at + 1..at + 4) == Some(delimiter.as_slice()))
    })
}

fn wrap_lines(raw: &str, source_at: usize, quote: u8) -> StringValue {
    let mut first = None;
    let mut output = None;
    let mut at = 0;
    while at < raw.len() {
        let (end, next) = next_line(raw, at);
        let mut begin = at;
        let mut finish = end;
        while begin < finish && matches!(raw.as_bytes()[begin], b' ' | b'\t') {
            begin += 1;
        }
        while finish > begin && matches!(raw.as_bytes()[finish - 1], b' ' | b'\t') {
            finish -= 1;
        }
        if begin < finish {
            if let Some((first_begin, first_end)) = first {
                let value = output.get_or_insert_with(|| {
                    let mut value = String::with_capacity(raw.len());
                    append_triple_text(&mut value, &raw[first_begin..first_end], quote);
                    value
                });
                value.push(' ');
                append_triple_text(value, &raw[begin..finish], quote);
            } else {
                first = Some((begin, finish));
            }
        }
        at = next;
    }
    if let Some(output) = output {
        return StringValue::Decoded(output);
    }
    let (begin, end) = first.unwrap_or((0, 0));
    let text = &raw[begin..end];
    if triple_needs_decoding(text, quote) {
        let mut output = String::with_capacity(text.len());
        append_triple_text(&mut output, text, quote);
        StringValue::Decoded(output)
    } else {
        StringValue::Source(Span {
            start: source_at + begin,
            end: source_at + end,
        })
    }
}

fn dedent_and_join(raw: &str, source_at: usize, quote: u8) -> Result<StringValue, ParseError> {
    let bytes = raw.as_bytes();
    let mut margin = usize::MAX;
    let mut at = 0;
    while at < bytes.len() {
        let (end, next) = next_line(raw, at);
        let mut spaces = at;
        while spaces < end && bytes[spaces] == b' ' {
            spaces += 1;
        }
        if bytes[spaces..end]
            .iter()
            .any(|b| !matches!(b, b' ' | b'\t'))
        {
            margin = margin.min(spaces - at);
        }
        at = next;
    }
    if margin == usize::MAX {
        margin = 0;
    }

    let unchanged = margin == 0
        && !raw.contains('\r')
        && (!raw.contains('\\')
            || (!triple_needs_decoding(raw, quote)
                && !raw
                    .split('\n')
                    .any(|line| line.trim_end_matches([' ', '\t']).ends_with('\\'))));
    if unchanged {
        return Ok(StringValue::Source(Span {
            start: source_at,
            end: source_at + raw.len(),
        }));
    }

    let mut output = String::with_capacity(raw.len());
    at = 0;
    let mut joined = false;
    while at < bytes.len() {
        let (end, next) = next_line(raw, at);
        let mut begin = at;
        let mut removed = 0;
        while begin < end && removed < margin && bytes[begin] == b' ' {
            begin += 1;
            removed += 1;
        }
        if joined {
            while begin < end && matches!(bytes[begin], b' ' | b'\t') {
                begin += 1;
            }
        }
        joined = false;
        let mut finish = end;
        let mut slash_start = finish;
        while slash_start > begin && bytes[slash_start - 1] == b'\\' {
            slash_start -= 1;
        }
        let slash_count = finish - slash_start;
        if next > end && slash_count % 2 == 1 {
            finish -= 1;
            joined = true;
        } else {
            let mut htail = end;
            while htail > begin && matches!(bytes[htail - 1], b' ' | b'\t') {
                htail -= 1;
            }
            let mut before_tail = htail;
            while before_tail > begin && bytes[before_tail - 1] == b'\\' {
                before_tail -= 1;
            }
            if htail != end && (htail - before_tail) % 2 == 1 {
                return Err(ParseError::new(
                    ErrorKind::InvalidEscape,
                    source_at + before_tail,
                    source_at + next,
                ));
            }
            if next == end && slash_count % 2 == 1 {
                return Err(ParseError::new(
                    ErrorKind::InvalidEscape,
                    source_at + end - 1,
                    source_at + end,
                ));
            }
        }
        append_triple_text(&mut output, &raw[begin..finish], quote);
        if next > end && !joined {
            output.push('\n');
        }
        at = next;
    }
    Ok(StringValue::Decoded(output))
}
