//! Complete consumer documents, source locations, and parser resource boundaries.

use daml::{ErrorKind, Limits, Node, Span, Value, parse, parse_with_limits};

#[test]
fn complete_application_retains_values_tags_and_original_source() {
    let source = include_str!("../examples/application.daml");
    let document = parse(source).unwrap();
    let root = document.root();
    assert_eq!(document.tag(root), Some("demo.Application"));

    let adapter = document.get(root, "adapter").unwrap();
    assert_eq!(document.tag(adapter), Some("network.Adapter"));
    assert!(matches!(&adapter.value, Value::Mapping(entries) if entries.is_empty()));

    let Value::Sequence(tasks) = &document.get(root, "tasks").unwrap().value else {
        panic!("tasks must be a sequence");
    };
    assert_eq!(document.tag(&tasks[0]), Some("demo.Task"));
    let request = document.get(&tasks[0], "request").unwrap();
    assert_eq!(document.tag(request), Some("text.Request"));
    let instruction = document.get(request, "instruction").unwrap();
    assert_eq!(
        document.string(instruction),
        Some("Extract the release name. Return JSON only.")
    );
    let context = document.get(request, "context").unwrap();
    assert_eq!(document.string(context), Some("release:\n  name: daml"));
    assert!(document.slice(context.span).unwrap().starts_with("\"\"\""));
    assert!(document.slice(context.span).unwrap().ends_with("\"\"\""));

    let ports = document.get(root, "ports").unwrap();
    assert_eq!(document.tag(ports), Some("network.Ports"));
    let Value::Sequence(items) = &ports.value else {
        panic!("ports must be a sequence")
    };
    assert_eq!(document.tag(&items[0]), Some("network.Port"));
    let Value::Number(span) = items[0].value else {
        panic!("port must be numeric")
    };
    assert_eq!(document.slice(span), Some("8080"));
    let Value::Number(span) = document.get(root, "exact").unwrap().value else {
        panic!("exact must retain a numeric lexeme");
    };
    assert_eq!(document.slice(span), Some("9007199254740993"));
    assert_eq!(
        document.get(root, "enabled").unwrap().value,
        Value::Bool(true)
    );
    assert_eq!(document.get(root, "optional").unwrap().value, Value::Null);
}

#[test]
fn document_outlives_input_and_checked_spans_cannot_split_utf8() {
    let document = {
        let source = String::from("{ greeting: \"héllo 🌍\" }");
        parse(source).unwrap()
    };
    let greeting = document.get(document.root(), "greeting").unwrap();
    assert_eq!(document.string(greeting), Some("héllo 🌍"));
    let accented = document.source().find('é').unwrap();
    assert_eq!(
        document.slice(Span {
            start: accented + 1,
            end: accented + 2
        }),
        None
    );
    assert_eq!(document.slice(Span { start: 4, end: 2 }), None);
    assert_eq!(
        document.slice(Span {
            start: 0,
            end: usize::MAX
        }),
        None
    );
}

#[test]
fn invalid_input_never_gets_a_guessed_interpretation() {
    for source in [
        "",
        "# only a comment",
        "{\"key\": 1}",
        "{key with space: 1}",
        "{key:}",
        "{key: NO}",
        "{key: 2026-10-17}",
        "{key: NaN}",
        "[1, 2]",
        "[1 2]",
        "{a: 1 b: 2}",
        "{a: 1\na: 2}",
        "---\n{}",
        "key: 1",
        "{x: &anchor 1}",
        "{x: *anchor}",
        "{x: |-\n  data\n}",
        "{x: >\n  data\n}",
        "null null",
        "!one !two {}",
        "! {}",
        "[}",
        "{a: [1\n2}",
    ] {
        let error = parse(source).expect_err(source);
        assert!(
            source.get(error.span.start..error.span.end).is_some(),
            "{source:?}: {error:?}"
        );
    }
}

#[test]
fn literal_indentation_does_not_change_mapping_ownership() {
    let document = parse("{\n adapter: !network.Adapter\n       timeout: 30\n}").unwrap();
    let adapter = document.get(document.root(), "adapter").unwrap();
    assert!(matches!(&adapter.value, Value::Mapping(entries) if entries.is_empty()));
    assert!(document.get(adapter, "timeout").is_none());
    let Value::Number(span) = document.get(document.root(), "timeout").unwrap().value else {
        panic!("timeout belongs to the outer mapping");
    };
    assert_eq!(document.slice(span), Some("30"));
}

#[test]
fn resource_bounds_have_exact_edges_and_no_unbounded_depth_override() {
    let scalar_limits = Limits {
        max_scalar_bytes: 3,
        ..Limits::default()
    };
    assert_eq!(
        parse_with_limits("123", scalar_limits)
            .unwrap()
            .root()
            .value,
        Value::Number(Span { start: 0, end: 3 })
    );
    assert_eq!(
        parse_with_limits("1234", scalar_limits).unwrap_err().kind,
        ErrorKind::LimitExceeded
    );
    let input_limits = Limits {
        max_input_bytes: 4,
        ..Limits::default()
    };
    assert_eq!(
        parse_with_limits("true", input_limits)
            .unwrap()
            .root()
            .value,
        Value::Bool(true)
    );
    assert_eq!(
        parse_with_limits("false", input_limits).unwrap_err().kind,
        ErrorKind::LimitExceeded
    );
    let node_limits = Limits {
        max_nodes: 2,
        ..Limits::default()
    };
    assert!(matches!(
        parse_with_limits("[1]", node_limits).unwrap().root().value,
        Value::Sequence(_)
    ));
    assert_eq!(
        parse_with_limits("[1\n2]", node_limits).unwrap_err().kind,
        ErrorKind::LimitExceeded
    );
    let depth_limits = Limits {
        max_depth: 2,
        ..Limits::default()
    };
    assert!(matches!(
        parse_with_limits("[[0]]", depth_limits)
            .unwrap()
            .root()
            .value,
        Value::Sequence(_)
    ));
    assert_eq!(
        parse_with_limits("[[[0]]]", depth_limits).unwrap_err().kind,
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        parse_with_limits(
            "[]",
            Limits {
                max_depth: 129,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidLimits
    );
    let permitted = format!("{}0{}", "[".repeat(128), "]".repeat(128));
    assert!(matches!(
        parse(&permitted).unwrap().root().value,
        Value::Sequence(_)
    ));
    let excessive = format!("[{permitted}]");
    assert_eq!(parse(excessive).unwrap_err().kind, ErrorKind::LimitExceeded);
}

#[test]
fn every_reachable_span_resolves_in_the_original_unicode_document() {
    fn check(document: &daml::Document, node: &Node) {
        assert!(document.slice(node.span).is_some());
        if let Some(tag) = node.tag {
            assert!(document.slice(tag).is_some());
        }
        match &node.value {
            Value::Mapping(entries) => {
                for entry in entries {
                    assert!(document.slice(entry.key).is_some());
                    check(document, &entry.value);
                }
            }
            Value::Sequence(items) => {
                for item in items {
                    check(document, item);
                }
            }
            Value::Number(span) => assert!(document.slice(*span).is_some()),
            Value::String(_) => assert!(document.string(node).is_some()),
            _ => {}
        }
    }
    let document = parse("# 前言\r\n!demo.Root {\r\n text: \"🌍\"\r\n list: [\r\n !demo.Text \"é\"\r\n 1.25e+3\r\n ]\r\n}").unwrap();
    check(&document, document.root());
    let list = document.get(document.root(), "list").unwrap();
    let Value::Sequence(items) = &list.value else {
        panic!("list")
    };
    assert_eq!(document.slice(items[0].span), Some("!demo.Text \"é\""));
}
