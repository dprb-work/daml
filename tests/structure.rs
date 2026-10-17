//! Structural grammar, literal names, and explicit failure boundaries.

use daml::{ErrorKind, Limits, Value, parse, parse_with_limits};

#[test]
fn parses_ordered_explicit_values_and_literal_names() {
    let document = parse("{\n  a.b/c: 123456789012345678901234567890.00e+10\n  enabled: true\n  absent: null\n  list: [1\n2]\n}").unwrap();
    let root = document.root();
    let Value::Mapping(entries) = &root.value else {
        panic!("mapping expected")
    };
    assert_eq!(entries.len(), 4);
    assert_eq!(document.slice(entries[0].key), Some("a.b/c"));
    let Value::Number(span) = &document.get(root, "a.b/c").unwrap().value else {
        panic!("number expected")
    };
    assert_eq!(
        document.slice(*span),
        Some("123456789012345678901234567890.00e+10")
    );
    assert_eq!(
        &document.get(root, "enabled").unwrap().value,
        &Value::Bool(true)
    );
    assert_eq!(&document.get(root, "absent").unwrap().value, &Value::Null);
}

#[test]
fn tags_bind_only_to_same_line_payload_and_bare_tags_are_maps() {
    let document =
        parse("[!outer {\nchild: !inner {}\n}\n!bare\n!commented # trailing comment\n]").unwrap();
    let Value::Sequence(items) = &document.root().value else {
        panic!("sequence expected")
    };
    assert_eq!(document.tag(&items[0]), Some("outer"));
    let Value::Mapping(entries) = &items[0].value else {
        panic!("mapping expected")
    };
    assert_eq!(document.tag(&entries[0].value), Some("inner"));
    assert!(matches!(&items[1].value, Value::Mapping(entries) if entries.is_empty()));
    assert!(matches!(&items[2].value, Value::Mapping(entries) if entries.is_empty()));
}

#[test]
fn rejects_ambiguous_delimiters_trailing_input_and_duplicate_keys() {
    for (source, kind) in [
        ("[1,2]", ErrorKind::ExpectedSeparator),
        ("[1 2]", ErrorKind::ExpectedSeparator),
        ("{a: 1\na: 2}", ErrorKind::DuplicateKey),
        ("1 false", ErrorKind::TrailingInput),
        ("{a:}", ErrorKind::ExpectedValue),
    ] {
        assert_eq!(parse(source).unwrap_err().kind, kind, "{source}");
    }
}

#[test]
fn enforces_limits_and_keeps_error_spans_on_utf8_boundaries() {
    let limits = Limits {
        max_nodes: 2,
        ..Limits::default()
    };
    assert_eq!(
        parse_with_limits("[1\n2\n3]", limits).unwrap_err().kind,
        ErrorKind::LimitExceeded
    );

    let limits = Limits {
        max_depth: 129,
        ..Limits::default()
    };
    assert_eq!(
        parse_with_limits("null", limits).unwrap_err().kind,
        ErrorKind::InvalidLimits
    );

    let error = parse("[é,]").unwrap_err();
    assert!("[é,]".get(error.span.start..error.span.end).is_some());
}
#[test]
fn rejects_controls_in_comments_and_names_and_noninitial_boms() {
    assert_eq!(
        parse("null # bad\u{0001} control").unwrap_err().kind,
        ErrorKind::InvalidCharacter
    );
    assert_eq!(
        parse("{bad\u{0001}key: null}").unwrap_err().kind,
        ErrorKind::InvalidCharacter
    );
    assert_eq!(
        parse("!bad\u{0001}tag null").unwrap_err().kind,
        ErrorKind::InvalidCharacter
    );
    assert_eq!(
        parse("null\n\u{feff}").unwrap_err().kind,
        ErrorKind::InvalidCharacter
    );
    assert_eq!(
        parse("{key\u{feff}: null}").unwrap_err().kind,
        ErrorKind::InvalidCharacter
    );
}

#[test]
fn rejects_invalid_decimal_forms_without_rounding_valid_lexemes() {
    for source in ["01", "1.", "1e", "-", ".5"] {
        assert_eq!(
            parse(source).unwrap_err().kind,
            ErrorKind::InvalidNumber,
            "{source}"
        );
    }
    let document = parse("-0.00000000000000000001E-999").unwrap();
    let Value::Number(span) = document.root().value else {
        panic!("decimal spelling must remain numeric");
    };
    assert_eq!(document.slice(span), Some("-0.00000000000000000001E-999"));
}
