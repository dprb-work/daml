//! Integration coverage for ordinary and triple-string parsing and decoding.
use daml::{ErrorKind, Limits, StringValue, Value, parse, parse_with_limits};

fn root_string(source: &str) -> String {
    let document = parse(source).unwrap();
    document.string(document.root()).unwrap().to_owned()
}

#[test]
fn ordinary_strings_decode_json_escapes_and_unicode() {
    assert_eq!(root_string(r#""plain café""#), "plain café");
    assert_eq!(
        root_string(r#""a\"\\\/\b\f\n\r\t""#),
        "a\"\\/\u{8}\u{c}\n\r\t"
    );
    assert_eq!(root_string(r#""\u0041\uD83D\uDE03""#), "A😃");
    let document = parse(r#""unchanged""#).unwrap();
    assert!(matches!(
        &document.root().value,
        Value::String(StringValue::Source(_))
    ));
}

#[test]
fn ordinary_string_errors_keep_original_byte_locations() {
    for (source, kind) in [
        (r#""\q""#, ErrorKind::InvalidEscape),
        (r#""\uD800""#, ErrorKind::InvalidEscape),
        (r#""\uDC00""#, ErrorKind::InvalidEscape),
        (r#""\uD800\u0041""#, ErrorKind::InvalidEscape),
        ("\"line\nbreak\"", ErrorKind::InvalidString),
        (r#""unfinished"#, ErrorKind::UnterminatedString),
    ] {
        let error = parse(source).unwrap_err();
        assert_eq!(error.kind, kind, "source: {source:?}");
        assert!(error.span.start <= source.len());
        assert!(error.span.end <= source.len());
    }
}

#[test]
fn invalid_continuations_and_scalar_limit_are_reported() {
    for source in [
        "\"\"\"\n  ends with slash \\\n\"\"\"",
        "\"\"\"\n  slash \\ \n  next\n\"\"\"",
        "\"\"\"\n  final slash \\ \n\"\"\"",
        "\"\"\"\n  final slash \\\t\n\"\"\"",
    ] {
        assert_eq!(parse(source).unwrap_err().kind, ErrorKind::InvalidEscape);
    }
    let error = parse("\"\u{e9}\\q\"").unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidEscape);
    assert_eq!((error.span.start, error.span.end), (3, 5));

    let limits = Limits {
        max_scalar_bytes: 2,
        ..Limits::default()
    };
    assert_eq!(
        parse_with_limits("\"x\"", limits).unwrap_err().kind,
        ErrorKind::LimitExceeded
    );
}

#[test]
fn error_spans_are_utf8_boundaries_for_limited_and_invalid_strings() {
    let mut limits = Limits {
        max_scalar_bytes: 1,
        ..Limits::default()
    };
    let source = "\"é\"";
    let error = parse_with_limits(source, limits).unwrap_err();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);
    assert!(source.is_char_boundary(error.span.start));
    assert!(source.is_char_boundary(error.span.end));

    limits.max_scalar_bytes = 6;
    let source = "\"\"\"\n  é\n\"\"\"";
    let error = parse_with_limits(source, limits).unwrap_err();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);
    assert!(source.is_char_boundary(error.span.start));
    assert!(source.is_char_boundary(error.span.end));

    for source in ["\"\\é\"", "\"\\ué\""] {
        let error = parse(source).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidEscape);
        assert!(source.is_char_boundary(error.span.start));
        assert!(source.is_char_boundary(error.span.end));
    }
}

#[test]
fn triple_strings_reject_raw_ascii_controls() {
    for source in ["\"\"\"\n  a\0b\n\"\"\"", "'''\n  a\u{1}b\n'''"] {
        let error = parse(source).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidString);
        assert!(source.is_char_boundary(error.span.start));
        assert!(source.is_char_boundary(error.span.end));
    }
}

#[test]
fn triple_double_dedents_normalizes_and_excludes_boundaries() {
    let source = "\"\"\"  \r\n        alpha\r\n          beta\r\n        \r\n          \r\n        \"\"\" # end\r\n";
    assert_eq!(root_string(source), "alpha\n  beta\n\n  ");
}

#[test]
fn triple_single_trims_drops_blank_lines_and_wraps() {
    let source = "'''\n  first \t\n \t\n second\r\n third \n'''";
    assert_eq!(root_string(source), "first second third");
    assert_eq!(root_string("'''\n\t\n   \n'''"), "");
}

#[test]
fn escaped_delimiters_and_backslashes_obey_parity() {
    assert_eq!(root_string("\"\"\"\n  a \\\"\"\" b\n\"\"\""), "a \"\"\" b");
    assert_eq!(
        root_string(
            r"'''
  a \''' b
'''"
        ),
        "a ''' b"
    );
    assert_eq!(
        root_string("\"\"\"\n  two \\\\ slashes\n\"\"\""),
        "two \\ slashes"
    );
    assert_eq!(root_string("'''\n two \\\\ slashes\n'''"), "two \\ slashes");
}

#[test]
fn triple_double_continuation_happens_after_dedent() {
    let source = "\"\"\"\n        left \\\n    right\n        end\n\"\"\"";
    assert_eq!(root_string(source), "    left right\n    end");
}

#[test]
fn blank_lines_tabs_and_all_blank_dedent_are_literal() {
    assert_eq!(
        root_string("\"\"\"\n    one\n\n      \n    two\n\"\"\""),
        "one\n\n  \ntwo"
    );
    assert_eq!(root_string("\"\"\"\n\tdata\n\"\"\""), "\tdata");
    assert_eq!(root_string("\"\"\"\n  \n\t \n\"\"\""), "  \n\t ");
}

#[test]
fn triple_delimiter_must_occupy_a_delimiter_line() {
    for source in [
        "\"\"\"\ntext \"\"\" trailing\n\"\"\"",
        "\"\"\"\n\"\"\"suffix\n",
    ] {
        assert_eq!(parse(source).unwrap_err().kind, ErrorKind::InvalidString);
    }
    let source = "\"\"\"\nvalue\n  \"\"\" # comment";
    assert_eq!(
        parse(source).unwrap().root().span.end,
        source.find("\"\"\" #").unwrap() + 3
    );
}

#[test]
fn string_decoding_is_independent_of_embedding_depth() {
    let flat = parse("\"\"\"\n  same\n  value\n\"\"\"").unwrap();
    let nested = parse("[{text: \"\"\"\n      same\n      value\n    \"\"\"\n}]").unwrap();
    let Value::Sequence(items) = &nested.root().value else {
        panic!("expected sequence")
    };
    let Value::Mapping(entries) = &items[0].value else {
        panic!("expected mapping")
    };
    assert_eq!(flat.string(flat.root()), nested.string(&entries[0].value));
}

#[test]
fn wrapped_strings_keep_comments_and_syntax_characters_as_data() {
    assert_eq!(root_string("'''\n # { !tag }\n'''"), "# { !tag }");
}

#[test]
fn closing_boundary_preserves_escaped_slashes_and_rejects_continuations() {
    for eol in ["\n", "\r\n", "\r"] {
        for tail in ["", " ", "\t"] {
            for (slashes, decoded) in [
                (r"\", None),
                (r"\\", Some(r"\")),
                (r"\\\", None),
                (r"\\\\", Some(r"\\")),
            ] {
                let source = format!("\"\"\"{eol}text{slashes}{tail}{eol}\"\"\"");
                if let Some(decoded) = decoded {
                    assert_eq!(root_string(&source), format!("text{decoded}{tail}"));
                } else {
                    let error = parse(&source).unwrap_err();
                    assert_eq!(error.kind, ErrorKind::InvalidEscape);
                    assert!(source.get(error.span.start..error.span.end).is_some());
                }
            }
        }
    }
}

#[test]
fn unchanged_literal_and_single_wrapped_lines_keep_exact_text() {
    for (source, expected) in [
        ("\"\"\"\nalpha\nbeta\n\"\"\"", "alpha\nbeta"),
        ("\"\"\"\n\\n remains literal\n\"\"\"", "\\n remains literal"),
        ("'''\n  \n  one line \t\n\n'''", "one line"),
        ("'''\n  a \\q remains literal\n'''", "a \\q remains literal"),
    ] {
        assert_eq!(root_string(source), expected);
    }
}
