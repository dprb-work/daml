//! Definition syntax, bounded parsing, and meaning-preserving reflow.

use daml::{
    ErrorKind, FormatOptions, Limits, Span, format_definition, parse, parse_definition,
    parse_definition_with_limits,
};

#[test]
fn default_objects_reject_nested_definition_declarations_at_their_source() {
    let source = "{value: !Object {} = { retries = 3 }}";
    let error = parse_definition(source).unwrap_err();
    let start = source.find("= 3").unwrap();
    assert_eq!(error.kind, ErrorKind::ExpectedColon);
    assert_eq!(
        error.span,
        Span {
            start,
            end: start + 1
        }
    );
}

#[test]
fn unicode_numeric_keys_preserve_exact_spelling_without_normalization() {
    let document = parse(
        "{200: \"ok\"\nnamé: \"value\"\né: \"composed\"\né: \"decomposed\"\n\u{1e4d0}: \"modern\"}",
    )
    .unwrap();
    for (key, expected) in [
        ("200", "ok"),
        ("namé", "value"),
        ("é", "composed"),
        ("é", "decomposed"),
        ("\u{1e4d0}", "modern"),
    ] {
        assert_eq!(
            document.string(document.get(document.root(), key).unwrap()),
            Some(expected)
        );
    }
}

#[test]
fn native_constraints_enforce_node_and_recursive_syntax_bounds() {
    let error = parse_definition_with_limits(
        "!Int | !Int | !Int | !Int | !Int | !Int",
        Limits {
            max_nodes: 4,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);

    let source = format!("{}!Object", "closed ".repeat(16));
    let error = parse_definition_with_limits(
        source,
        Limits {
            max_depth: 8,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::LimitExceeded);
}
#[test]
fn explicit_tuple_constraints_are_formatted_as_native_payloads() {
    let definition = parse_definition("!Tuple [!Int\n!String]").unwrap();
    let output = format_definition(&definition, FormatOptions::default()).unwrap();
    assert_eq!(output, "!Tuple [\n  !Int\n  !String\n]\n");
    let reparsed = parse_definition(&output).unwrap();
    assert_eq!(
        format_definition(&reparsed, FormatOptions::default()).unwrap(),
        output
    );
}

#[test]
fn bare_array_data_remains_formatable_inside_a_definition() {
    let definition = parse_definition("[1.00\n!Tag \"value\"]").unwrap();
    let output = format_definition(&definition, FormatOptions::default()).unwrap();
    assert!(output.contains("1.00"));
    assert!(output.contains("!Tag \"value\""));
    let reparsed = parse_definition(&output).unwrap();
    assert_eq!(
        format_definition(&reparsed, FormatOptions::default()).unwrap(),
        output
    );
}

#[test]
fn explicit_tuple_formatting_preserves_comments_and_default_spelling() {
    let source = "!Tuple [ # positions\n  !Int # first\n  !String\n] = [1.00\n\"x\"]";
    let definition = parse_definition(source).unwrap();
    let output = format_definition(&definition, FormatOptions::default()).unwrap();
    assert!(output.contains("# positions"));
    assert!(output.contains("# first"));
    assert!(output.contains("1.00"));
    assert!(output.starts_with("!Tuple ["));
    let reparsed = parse_definition(&output).unwrap();
    assert_eq!(
        format_definition(&reparsed, FormatOptions::default()).unwrap(),
        output
    );
}

#[test]
fn typed_regex_payloads_are_raw_and_stable_under_formatting() {
    let source = r#"!String { pattern: !Regex "\d+\s+\p{Greek}" }"#;
    let definition = parse_definition(source).unwrap();
    let output = format_definition(&definition, FormatOptions::default()).unwrap();
    assert_eq!(
        output,
        r#"!String { pattern: !Regex "\d+\s+\p{Greek}" }
"#
    );
    let reparsed = parse_definition(&output).unwrap();
    assert_eq!(
        format_definition(&reparsed, FormatOptions::default()).unwrap(),
        output
    );
}

#[test]
fn formatter_separates_regex_core_from_a_syntax_only_default() {
    let source = r#"!Regex "a" = unset"#;
    let definition = parse_definition(source).unwrap();
    assert_eq!(
        format_definition(&definition, FormatOptions::default()).unwrap(),
        "!Regex \"a\" = unset\n"
    );
}

#[test]
fn grouped_comments_survive_native_reflow() {
    let source = "{value: ( # before\n!Int)}";
    let definition = parse_definition(source).unwrap();
    let output = format_definition(&definition, FormatOptions::default()).unwrap();
    assert_ne!(output, source);
    assert!(output.contains("# before"));
    let reparsed = parse_definition(&output).unwrap();
    assert_eq!(
        format_definition(&reparsed, FormatOptions::default()).unwrap(),
        output
    );
}

#[test]
fn formatter_reflows_native_members_and_is_idempotent() {
    let definition = parse_definition("{short: !Int\nlong_member: !String}").unwrap();
    let options = FormatOptions { line_width: 16 };
    let output = format_definition(&definition, options).unwrap();
    assert_eq!(output, "{\n  short: !Int\n  long_member:\n    !String\n}\n");
    let reparsed = parse_definition(&output).unwrap();
    assert_eq!(format_definition(&reparsed, options).unwrap(), output);
}

#[test]
fn formatter_preserves_decoded_literal_data_and_comments() {
    let source = "{\n  value: \"\"\"\n  first\n  \"\"\" # tail\n}";
    let definition = parse_definition(source).unwrap();
    let options = FormatOptions::default();
    let output = format_definition(&definition, options).unwrap();
    assert!(output.contains("# tail"));
    let before = parse(source).unwrap();
    let after = parse(&output).unwrap();
    assert_eq!(
        before.string(before.get(before.root(), "value").unwrap()),
        Some("first")
    );
    assert_eq!(
        after.string(after.get(after.root(), "value").unwrap()),
        Some("first")
    );
    assert_eq!(
        format_definition(&parse_definition(&output).unwrap(), options).unwrap(),
        output
    );
}

#[test]
fn grouping_a_default_does_not_make_an_optional_default_declaration_valid() {
    let error = parse_definition("{ retries?: (!Object = {}) }").unwrap_err();
    assert_eq!(error.kind, ErrorKind::UnexpectedToken);
}

#[test]
fn repeated_default_assignment_is_rejected_before_formatting() {
    let source = "!Object { x: !Int } = { x: 1 } = { x: 2 }";
    let error = parse_definition(source).unwrap_err();
    let start = source.rfind('=').unwrap();
    assert_eq!(
        error.span,
        Span {
            start,
            end: start + 1
        }
    );
}

#[test]
fn ordinary_record_members_reject_scalar_defaults_even_through_composition() {
    for source in [
        "{ retries = 3 }",
        "{ retries: !Int = 3 }",
        "{ retries: (!Int = 3) }",
        "{ retries: (!Int = 3) | !String }",
        "{ settings = {} }",
    ] {
        let error = parse_definition(source).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnexpectedToken, "{source}");
    }
}

#[test]
fn formatter_preserves_comments_between_closed_and_its_operand() {
    let source = "closed # Keep this explanation\n!Object {}";
    let formatted =
        format_definition(&parse_definition(source).unwrap(), FormatOptions::default()).unwrap();
    assert_eq!(formatted, "closed # Keep this explanation\n!Object {}\n");
    assert_eq!(
        format_definition(
            &parse_definition(&formatted).unwrap(),
            FormatOptions::default()
        )
        .unwrap(),
        formatted
    );
}
