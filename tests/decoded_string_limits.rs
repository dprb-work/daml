//! An unterminated decoded triple cannot hide scalar exhaustion at its final EOL.

use daml::{ErrorKind, Limits, parse_definition_with_limits, parse_with_limits};

fn check_tail_budget(source: &str, token_bytes: usize, syntax_error: ErrorKind) {
    for (cap, expected) in [
        (token_bytes - 1, ErrorKind::LimitExceeded),
        (token_bytes, syntax_error),
    ] {
        let limits = Limits {
            max_input_bytes: source.len(),
            max_scalar_bytes: cap,
            ..Limits::default()
        };
        // Both public data admission and native tooling use the decoded scanner.
        for error in [
            parse_with_limits(source, limits).unwrap_err(),
            parse_definition_with_limits(source, limits).unwrap_err(),
        ] {
            assert_eq!(error.kind, expected, "source {source:?}, scalar cap {cap}");
            assert!(source.get(error.span.start..error.span.end).is_some());
        }
    }
}

#[test]
fn unterminated_ordinary_triple_tail_reports_scalar_exhaustion_before_syntax() {
    for delimiter in ["\"\"\"", "'''"] {
        for eol in ["\n", "\r\n", "\r"] {
            // The LF double-quoted case is exactly seven source bytes with cap 6.
            let source = format!("{delimiter}{eol}ab{eol}");
            check_tail_budget(&source, source.len(), ErrorKind::UnterminatedString);
        }
    }
}

#[test]
fn unterminated_temporal_triple_tail_does_not_remap_scalar_exhaustion() {
    for constructor in ["Date", "Time", "DateTime"] {
        for delimiter in ["\"\"\"", "'''"] {
            for eol in ["\n", "\r\n", "\r"] {
                // Keep the payload cap above constructor-name admission, so
                // DateTime also discriminates the quoted-token budget boundary.
                let body = if constructor == "DateTime" {
                    "abcdefgh"
                } else {
                    "ab"
                };
                let token = format!("{delimiter}{eol}{body}{eol}");
                let source = format!("!{constructor} {token}");
                check_tail_budget(&source, token.len(), ErrorKind::InvalidTemporal);
            }
        }
    }
}
