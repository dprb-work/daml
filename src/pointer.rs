//! RFC 6901 paths shared by schema locations and instance diagnostics.
//! URI-fragment percent encoding belongs to reference resolution, not here.

use std::borrow::Cow;

pub(crate) fn join(parent: &str, segment: &str) -> String {
    let mut path = String::with_capacity(parent.len() + segment_len(segment));
    path.push_str(parent);
    append(&mut path, segment);
    path
}

pub(crate) fn segment_len(segment: &str) -> usize {
    1 + segment.len()
        + segment
            .bytes()
            .filter(|byte| matches!(*byte, b'~' | b'/'))
            .count()
}

/// Append after the caller has admitted/reserved the resulting path size.
pub(crate) fn append(path: &mut String, segment: &str) {
    path.push('/');
    for ch in segment.chars() {
        match ch {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            _ => path.push(ch),
        }
    }
}

pub(crate) fn decode_segment(part: &str) -> Option<Cow<'_, str>> {
    if !part.contains('~') {
        return Some(Cow::Borrowed(part));
    }
    let mut decoded = String::with_capacity(part.len());
    let mut chars = part.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            decoded.push(ch);
        } else {
            decoded.push(match chars.next()? {
                '0' => '~',
                '1' => '/',
                _ => return None,
            });
        }
    }
    Some(Cow::Owned(decoded))
}
