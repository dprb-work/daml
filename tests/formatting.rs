//! Reflow, comment retention, and semantic preservation across source-sensitive cases.

use daml::{Document, FormatError, FormatOptions, Node, Value, format, parse};

fn same_value(left: &Document, a: &Node, right: &Document, b: &Node) {
    assert_eq!(left.tag(a), right.tag(b));
    match (&a.value, &b.value) {
        (Value::Null, Value::Null) => {}
        (Value::Bool(a), Value::Bool(b)) => assert_eq!(a, b),
        (Value::Number(a), Value::Number(b)) => assert_eq!(left.slice(*a), right.slice(*b)),
        (Value::String(_), Value::String(_)) => assert_eq!(left.string(a), right.string(b)),
        (Value::Sequence(a), Value::Sequence(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                same_value(left, a, right, b);
            }
        }
        (Value::Mapping(a), Value::Mapping(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_eq!(left.slice(a.key), right.slice(b.key));
                same_value(left, &a.value, right, &b.value);
            }
        }
        _ => panic!("formatter changed the value kind"),
    }
}

fn checked(source: &str, width: usize) -> String {
    let before = parse(source).unwrap();
    let options = FormatOptions { line_width: width };
    let output = format(&before, options).unwrap();
    let after = parse(output.as_str()).unwrap_or_else(|error| panic!("{error}\n{output}"));
    same_value(&before, before.root(), &after, after.root());
    assert_eq!(
        format(&after, options).unwrap(),
        output,
        "formatting must be idempotent"
    );
    output
}

#[test]
fn single_entry_containers_expand_and_collapse_to_width() {
    let input = "{long_key:{value:true}}";
    assert_eq!(checked(input, 80), "{ long_key: { value: true } }\n");
    assert_eq!(
        checked(input, 24),
        "{\n  long_key: {\n    value: true\n  }\n}\n"
    );
}

#[test]
fn multiple_entries_keep_required_separators_even_at_large_width() {
    assert_eq!(
        checked("{a:1\nb:[2\n3]}", 1000),
        "{\n  a: 1\n  b: [\n    2\n    3\n  ]\n}\n"
    );
}

#[test]
fn tags_preserve_binding_and_explicit_empty_container_spelling() {
    assert_eq!(checked("!olaf.omp.adapter", 1), "!olaf.omp.adapter\n");
    assert_eq!(checked("!olaf.omp.adapter{}", 80), "!olaf.omp.adapter {}\n");
    checked("{adapter:!outer {child:!inner [!bare]}}", 12);
}

#[test]
fn comments_keep_text_order_and_inline_or_own_line_attachment() {
    let input = "# head\n{ # open\nkey: # label\n!T {x:1} # suffix\n# dangling\n} # tail";
    let output = checked(input, 80);
    assert_eq!(
        output,
        "# head\n{ # open\n  key: # label\n    !T { x: 1 } # suffix\n  # dangling\n} # tail\n"
    );
    assert_eq!(checked("[ # empty\n]", 80), "[ # empty\n]\n");
}

#[test]
fn escaped_strings_hash_characters_names_and_number_lexemes_survive() {
    let input = r#"{a.b/c: !λ "hash # quote \" and \u03bb"
number: -0.0000001E+0009}"#;
    let output = checked(input, 18);
    assert!(output.contains(r#""hash # quote \" and \u03bb""#));
    assert!(output.contains("-0.0000001E+0009"));
}

#[test]
fn block_string_blank_only_data_is_not_structurally_indented() {
    let input = "{text:\"\"\"\n  \t\n    \n  \"\"\"\n}";
    let output = checked(input, 80);
    let parsed = parse(output).unwrap();
    assert_eq!(
        parsed.string(parsed.get(parsed.root(), "text").unwrap()),
        Some("  \t\n    ")
    );
}

#[test]
fn literal_blank_boundaries_relative_indent_and_continuations_survive() {
    let input = r####"!root { text: """

    first
      relative
    long\
    line

"""
}"####;
    checked(input, 20);
}

#[test]
fn wrapped_and_escaped_delimiter_bodies_remain_opaque() {
    let input = r####"{literal: """
  escaped \""" text
"""
wrapped: '''
  first   word
  next\
  part
'''
}"####;
    checked(input, 16);
}

#[test]
fn empty_blocks_and_physical_line_endings_preserve_values() {
    for input in [
        "\"\"\"\n\"\"\"",
        "\"\"\"\n\n\"\"\"",
        "\"\"\"\r\n  x\r\n\"\"\"",
        "'''\r  x\ry\r'''",
    ] {
        checked(input, 80);
    }
}

#[test]
fn bom_and_comments_are_not_mistaken_for_scalar_content() {
    assert_eq!(
        checked("\u{feff}# pre\r\n!bare # post", 80),
        "\u{feff}# pre\n!bare # post\n"
    );
}

#[test]
fn invalid_width_is_a_structured_error() {
    let document = parse("!root { value: 1 }").unwrap();
    for line_width in [0, usize::MAX] {
        assert_eq!(
            format(&document, FormatOptions { line_width }),
            Err(FormatError::InvalidLineWidth)
        );
    }
}
