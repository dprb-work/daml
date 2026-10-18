//! Physical line-ending mechanics shared by the parser and source-preserving tools.

/// Return the byte after an LF, CRLF, or CR beginning at `at`.
pub(crate) fn line_end(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at) {
        Some(b'\n') => Some(at + 1),
        Some(b'\r') => Some(at + 1 + usize::from(bytes.get(at + 1) == Some(&b'\n'))),
        _ => None,
    }
}

/// Exclude one physical line ending immediately before the in-bounds offset `at`.
pub(crate) fn newline_start(bytes: &[u8], at: usize) -> usize {
    if at > 1 && bytes[at - 1] == b'\n' && bytes[at - 2] == b'\r' {
        at - 2
    } else if at > 0 && matches!(bytes[at - 1], b'\n' | b'\r') {
        at - 1
    } else {
        at
    }
}
