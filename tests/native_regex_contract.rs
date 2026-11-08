//! Public native-bundle matching, source fidelity, and bounded raw payloads.

use daml::{
    Bundle, ErrorKind, FormatOptions, Limits, Span, format_definition, parse, parse_definition,
    parse_definition_with_limits,
};

fn bundle(source: &str) -> Bundle {
    Bundle::from_sources(
        ".",
        [
            ("daml.bundle".to_owned(), "{}".to_owned()),
            ("Pattern.daml".to_owned(), source.to_owned()),
        ],
    )
    .unwrap()
}

fn check_matches(source: &str, cases: &[(&str, bool)]) {
    let bundle = bundle(source);
    let input = parse("\"\"").unwrap();
    let schemas = bundle.compile(&input, Some("self.Pattern")).unwrap();
    for &(value, expected) in cases {
        let input = parse(serde_json::to_string(value).unwrap()).unwrap();
        assert_eq!(
            schemas.resolve_as(&input, "self.Pattern").is_ok(),
            expected,
            "pattern source {source:?}, input {value:?}"
        );
    }
}

fn string_pattern(payload: &str) -> String {
    format!("!String {{ pattern: !Regex {payload}\n}}")
}

#[test]
fn raw_backslashes_are_regex_syntax_in_all_three_forms() {
    for payload in [
        "\"\\d+\\\\x\"".to_owned(),
        "\"\"\"\n  \\d+\\\\x\n  \"\"\"".to_owned(),
        "'''\n  \\d+\\\\x\n  '''".to_owned(),
    ] {
        check_matches(
            &string_pattern(&payload),
            &[
                ("12\\x", true),
                ("١٢\\x", true),
                ("12x", false),
                ("dd\\x", false),
            ],
        );
    }
}

#[test]
fn literal_layout_normalizes_line_endings_without_enabling_verbose_mode() {
    for eol in ["\n", "\r\n", "\r"] {
        let payload = format!("\"\"\"  {eol}    a{eol}      b{eol}    {eol}    c{eol}    \"\"\"");
        check_matches(
            &string_pattern(&payload),
            &[
                ("a\n  b\n\nc", true),
                ("abc", false),
                ("a b c", false),
                ("a\r\n  b\r\n\r\nc", false),
            ],
        );
        let payload = format!("'''{eol}    a \t{eol} \t{eol}  b {eol}    c{eol}  '''");
        check_matches(
            &string_pattern(&payload),
            &[("a b c", true), ("abc", false), ("a\nb\nc", false)],
        );
    }
}

#[test]
fn backslash_before_physical_newline_is_not_a_string_continuation() {
    for eol in ["\n", "\r\n", "\r"] {
        let payload = format!("\"\"\"{eol}  a\\{eol}  b{eol}  \"\"\"");
        check_matches(&string_pattern(&payload), &[("a\nb", true), ("ab", false)]);
        let folded = format!("'''{eol}  a\\{eol}  b{eol}  '''");
        check_matches(&string_pattern(&folded), &[("a b", true), ("ab", false)]);
        // Admission must also preserve a final unpaired slash, rather than treating
        // the closing boundary as an invalid ordinary-string continuation.
        parse_definition(format!("!Regex \"\"\"{eol}  a\\{eol}  \"\"\"")).unwrap();
    }
}

#[test]
fn verbose_comments_need_literal_line_breaks_and_full_matching_keeps_alternatives() {
    let literal = string_pattern(
        "\"\"\"\n  (?x)\n  a | abc # first alternative\n  d # final comment\n  \"\"\"",
    );
    check_matches(
        &literal,
        &[
            ("a", true),
            ("abcd", true),
            ("abc", false),
            ("zabcd", false),
            ("abcdz", false),
        ],
    );
    // Folding deliberately allows the first # comment to consume the remainder.
    let folded = string_pattern("'''\n  (?x)a # explanation\n  b\n  '''");
    check_matches(&folded, &[("a", true), ("ab", false)]);
    let literal = string_pattern("\"\"\"\n  (?x)a # explanation\n  b # final comment\n  \"\"\"");
    check_matches(
        &literal,
        &[("ab", true), ("a", false), ("zab", false), ("abz", false)],
    );
}

#[test]
fn delimiter_parity_is_lexical_and_does_not_consume_raw_slashes() {
    for delimiter in ["\"\"\"", "'''"] {
        for slashes in [1, 3] {
            let escaped = format!("a {}{delimiter} b", "\\".repeat(slashes));
            let source = string_pattern(&format!("{delimiter}\n  {escaped}\n  {delimiter}"));
            let definition = parse_definition(&source).unwrap();
            let output = format_definition(&definition, FormatOptions::default()).unwrap();
            assert!(
                output.contains(&escaped),
                "raw escaped delimiter lost: {output}"
            );
            let value = format!("a {}{delimiter} b", "\\".repeat((slashes - 1) / 2));
            check_matches(&source, &[(&value, true), ("a b", false)]);
        }
        // An even slash run does not escape a delimiter in the middle of a line.
        let source = format!("!Regex {delimiter}\n  a \\\\{delimiter} b\n  {delimiter}");
        assert_eq!(
            parse_definition(source).unwrap_err().kind,
            ErrorKind::InvalidString
        );
    }
}

#[test]
fn raw_single_line_admission_and_payload_only_budget_are_unchanged() {
    // Keep both caps >= the five-byte constructor name, so the boundary
    // discriminates payload admission rather than identifier admission.
    for payload in ["abcdef", "ééé", "\\d+abc", "\\\\abcd"] {
        let source = format!("!Regex \"{payload}\"");
        let limits = Limits {
            max_scalar_bytes: payload.len(),
            ..Limits::default()
        };
        parse_definition_with_limits(&source, limits).unwrap();
        let error = parse_definition_with_limits(
            &source,
            Limits {
                max_scalar_bytes: payload.len() - 1,
                ..limits
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::LimitExceeded);
        assert!(source.get(error.span.start..error.span.end).is_some());
    }
    // A quote is not escapable in legacy raw single-line Regex syntax.
    assert!(parse_definition("!Regex \"a\\\"b\"").is_err());
    for payload in ["a\nb", "a\rb", "a\0b", "a\tb", "a\u{7f}b", "a\u{feff}b"] {
        assert!(parse_definition(format!("!Regex \"{payload}\"")).is_err());
    }
}

#[test]
fn triple_budget_counts_the_original_quoted_token_before_layout_shrinks_it() {
    for token in [
        "\"\"\"\n  é\n  \"\"\"",
        "'''\r\n  \\d+ \r\n \t\r\n  '''",
        "\"\"\"\n  a \\\"\"\" b\n  \"\"\"",
        "\"\"\"\n                        a\n                        \"\"\"",
    ] {
        let source = format!("!Regex {token}");
        let limits = Limits {
            max_scalar_bytes: token.len(),
            ..Limits::default()
        };
        parse_definition_with_limits(&source, limits).unwrap();
        for cap in [token.len() - 1, 5] {
            let error = parse_definition_with_limits(
                &source,
                Limits {
                    max_scalar_bytes: cap,
                    ..limits
                },
            )
            .unwrap_err();
            assert_eq!(error.kind, ErrorKind::LimitExceeded);
            assert!(source.get(error.span.start..error.span.end).is_some());
        }
    }
}

#[test]
fn malformed_multiline_payloads_keep_original_utf8_source_locations() {
    for token in [
        "\"\"\"inline\n\"\"\"",
        "'''\n  é\0\n  '''",
        "\"\"\"\n  é\n  \"\"\"suffix",
        "'''\n  é",
    ] {
        let source = format!("# 😀 original\n!Regex {token}");
        let error = parse_definition(&source).unwrap_err();
        assert!(
            source.get(error.span.start..error.span.end).is_some(),
            "{error:?}"
        );
        assert!(
            error.span.start >= source.find("!Regex").unwrap(),
            "{error:?}"
        );
    }
    let source = "# 😀 original\n!Regex '''\n  é\0\n  '''";
    let error = parse_definition(source).unwrap_err();
    let start = source.find('\0').unwrap();
    assert_eq!(
        error.span,
        Span {
            start,
            end: start + 1
        }
    );
}

#[test]
fn regex_compile_diagnostics_reference_original_multiline_utf8_payload() {
    let source =
        "# 😀 original\n!String { pattern: !Regex \"\"\"\n    é(?=prefix).*\n    \"\"\"\n}";
    let error = match bundle(source).compile(&parse("\"\"").unwrap(), Some("self.Pattern")) {
        Err(error) => error,
        Ok(_) => panic!("unsupported look-around must fail compilation"),
    };
    assert!(
        error
            .path
            .as_deref()
            .expect("original definition source path")
            .ends_with("Pattern.daml")
    );
    let span = error.span.expect("source-backed regex error");
    assert!(source.get(span.start..span.end).is_some());
    assert!(span.start <= source.find("é(?=prefix)").unwrap());
    assert!(span.end >= source.find(".*").unwrap() + 2);
    assert!(span.start >= source.find("\"\"\"").unwrap() + 3);
    assert!(span.end <= source.rfind("\"\"\"").unwrap());
}

#[test]
fn formatting_preserves_raw_spelling_comments_and_matching_at_nested_widths() {
    for (delimiter, body, accepted, rejected) in [
        (
            "\"\"\"",
            "(?x)\\d+ # digits\n      \\\\x # final",
            "12\\x",
            "12x",
        ),
        ("'''", "\\d+\n      \\\\x", "12 \\x", "12\\x"),
    ] {
        let source = format!(
            "# head\n!Object {{ very_long_outer_member: !Object {{ code: !String {{\n    pattern: !Regex {delimiter}\n      {body}\n      {delimiter} # tail\n  }}\n}}\n}}"
        );
        let definition = parse_definition(&source).unwrap();
        assert_eq!(definition.source(), source);
        for width in [1, 18, 80] {
            let options = FormatOptions { line_width: width };
            let output = format_definition(&definition, options).unwrap();
            assert!(output.contains("# head"));
            assert!(output.contains("# tail"));
            assert!(output.contains("\\d+"));
            assert!(output.contains("\\\\x"));
            assert!(output.contains(delimiter));
            let reparsed = parse_definition(&output).unwrap();
            assert_eq!(format_definition(&reparsed, options).unwrap(), output);
            for text in [&source, &output] {
                let bundle = bundle(text);
                let input = |value: &str| {
                    parse(format!(
                        "{{ very_long_outer_member: {{ code: {} }} }}",
                        serde_json::to_string(value).unwrap()
                    ))
                    .unwrap()
                };
                let valid = input(accepted);
                let schemas = bundle.compile(&valid, Some("self.Pattern")).unwrap();
                assert!(schemas.resolve_as(&valid, "self.Pattern").is_ok());
                assert!(
                    schemas
                        .resolve_as(&input(rejected), "self.Pattern")
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn data_regex_tags_still_use_the_ordinary_string_decoder() {
    let document = parse("!Regex \"\\\\d+\"").unwrap();
    assert_eq!(document.string(document.root()), Some("\\d+"));
    assert!(parse("!Regex \"\\d+\"").is_err());
    let document = parse("!Regex \"\"\"\n  a \\\\ b\n  \"\"\"").unwrap();
    assert_eq!(document.string(document.root()), Some("a \\ b"));
}

#[test]
fn intrinsic_temporal_triples_keep_decoding_and_reflow_semantics() {
    for delimiter in ["\"\"\"", "'''"] {
        for (tag, value, expected) in [
            ("Date", "2024-02-29", "2024-02-29"),
            ("Time", "T09:30", "09:30:00"),
            ("DateTime", "2026-10-29T09Z", "2026-10-29T09:00:00Z"),
        ] {
            let source = format!(
                "{{ nested: {{ value: !{tag} {delimiter}\n  {value}\n  {delimiter}\n}}\n}}"
            );
            let definition = parse_definition(&source).unwrap();
            for width in [12, 80] {
                let options = FormatOptions { line_width: width };
                let output = format_definition(&definition, options).unwrap();
                let reparsed = parse_definition(&output).unwrap();
                assert_eq!(format_definition(&reparsed, options).unwrap(), output);
                for text in [&source, &output] {
                    let document = parse(text).unwrap();
                    let nested = document.get(document.root(), "nested").unwrap();
                    let value = document.get(nested, "value").unwrap();
                    assert_eq!(document.temporal(value).unwrap().as_str(), expected);
                }
            }
        }
    }
}

#[test]
fn multiline_constructor_boundary_and_empty_layout_follow_shared_rules() {
    for delimiter in ["\"\"\"", "'''"] {
        for gap in ["", " ", "\t"] {
            parse_definition(format!("!Regex{gap}{delimiter}\n  a\n  {delimiter}")).unwrap();
        }
        assert!(parse_definition(format!("!Regex\n{delimiter}\n  a\n  {delimiter}")).is_err());
        let token = format!("{delimiter}\n{delimiter}");
        check_matches(&string_pattern(&token), &[("", true), ("a", false)]);
    }
    check_matches(
        &string_pattern("\"\"\"\n  \n\t \n\"\"\""),
        &[("\n", true), ("", false), ("  ", false)],
    );
    check_matches(
        &string_pattern("'''\n  \n\t \n'''"),
        &[("", true), ("\n", false)],
    );
}
