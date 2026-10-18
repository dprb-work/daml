use daml::{
    Document, ErrorOrigin, Limits, Node, Origin, ResolutionLimits, ResolveErrorKind, ResolvedNode,
    ResolvedValue, SchemaModule, Schemas, Span, StringValue, Value, parse_with_limits,
};
use serde_json::json;
use std::{collections::HashSet, sync::LazyLock};

pub const PARSE_LIMITS: Limits = Limits {
    max_input_bytes: 64 * 1024,
    max_scalar_bytes: 16 * 1024,
    max_nodes: 4096,
    max_depth: 32,
};

pub const RESOLUTION_LIMITS: ResolutionLimits = ResolutionLimits {
    max_nodes: 4096,
    max_bytes: 128 * 1024,
    max_steps: 100_000,
};

pub const SCHEMA_INPUT_LIMIT: usize = 16 * 1024;

fn same_value(a: &Document, left: &Node, b: &Document, right: &Node) -> bool {
    if a.tag(left) != b.tag(right) {
        return false;
    }
    match (&left.value, &right.value) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => a.slice(*left) == b.slice(*right),
        (Value::String(_), Value::String(_)) => a.string(left) == b.string(right),
        (Value::Sequence(left), Value::Sequence(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same_value(a, left, b, right))
        }
        (Value::Mapping(left), Value::Mapping(right)) => {
            left.len() == right.len()
                && left.iter().zip(right).all(|(left, right)| {
                    a.slice(left.key) == b.slice(right.key)
                        && same_value(a, &left.value, b, &right.value)
                })
        }
        _ => false,
    }
}

fn check_span(source: &str, span: Span) {
    assert!(span.start <= span.end && span.end <= source.len());
    assert!(source.is_char_boundary(span.start) && source.is_char_boundary(span.end));
}

fn check_node(document: &Document, node: &Node) -> usize {
    check_span(document.source(), node.span);
    assert!(document.slice(node.span).is_some());
    if let Some(tag) = node.tag {
        check_span(document.source(), tag);
        assert!(node.span.start <= tag.start && tag.end <= node.span.end);
        assert!(document.tag(node).is_some());
    }
    let mut count = 1;
    match &node.value {
        Value::Number(span) | Value::String(StringValue::Source(span)) => {
            check_span(document.source(), *span);
            assert!(document.slice(*span).is_some());
        }
        Value::String(StringValue::Decoded(_)) | Value::Null | Value::Bool(_) => {}
        Value::Sequence(items) => {
            for child in items {
                assert!(node.span.start <= child.span.start && child.span.end <= node.span.end);
                count += check_node(document, child);
            }
        }
        Value::Mapping(entries) => {
            let mut keys = HashSet::with_capacity(entries.len());
            for entry in entries {
                check_span(document.source(), entry.key);
                assert!(node.span.start <= entry.key.start && entry.key.end <= node.span.end);
                assert!(
                    node.span.start <= entry.value.span.start
                        && entry.value.span.end <= node.span.end
                );
                assert!(keys.insert(document.slice(entry.key).unwrap()));
                count += check_node(document, &entry.value);
            }
        }
    }
    count
}

pub fn exercise_parse(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    if source.len() > PARSE_LIMITS.max_input_bytes {
        return;
    }
    let document = match parse_with_limits(source, PARSE_LIMITS) {
        Ok(document) => document,
        Err(error) => {
            check_span(source, error.span);
            return;
        }
    };
    assert_eq!(document.source(), source);
    assert!(check_node(&document, document.root()) <= PARSE_LIMITS.max_nodes);
    assert!(
        document
            .slice(Span {
                start: 0,
                end: source.len() + 1
            })
            .is_none()
    );
    assert!(
        document
            .slice(Span {
                start: source.len(),
                end: 0
            })
            .is_none()
    );
    if let Some((index, _)) = source.char_indices().find(|(_, ch)| !ch.is_ascii()) {
        assert!(
            document
                .slice(Span {
                    start: index,
                    end: index + 1
                })
                .is_none()
        );
    }
    if source.len() < PARSE_LIMITS.max_input_bytes {
        let mut appended = source.to_owned();
        appended.push('\n');
        let again = parse_with_limits(appended, PARSE_LIMITS)
            .expect("trailing trivia cannot invalidate a parsed value within the byte limit");
        assert!(same_value(&document, document.root(), &again, again.root()));
    }
}

struct NativeFixture {
    schemas: Schemas,
    known_tags: HashSet<String>,
}

fn native_fixture() -> &'static NativeFixture {
    static FIXTURE: LazyLock<NativeFixture> = LazyLock::new(|| {
        let config = json!({"types": {
            "fuzz.Config": {"schema": {
                "type": "object",
                "properties": {
                    "name": {"default": "fixture", "x-daml-default-tags": [{"path": "", "tag": "fuzz.Text"}]},
                    "count": {"type": "integer", "minimum": 0, "default": 7, "x-daml-default-tags": [{"path": "", "tag": "fuzz.Count"}]},
                    "items": {"type": "array", "items": {"type": "integer"}, "uniqueItems": true, "default": [1, 2]},
                    "nullable": {"default": null},
                    "adapter": {"x-daml-slot": "fuzz.Provider"},
                    "choice": {"anyOf": [{"type": "string", "default": "left"}, {"type": "string", "default": "right"}]}
                },
                "required": ["name", "count", "items"], "additionalProperties": true
            }},
            "fuzz.Ambiguous": {"schema": {"oneOf": [
                {"type": "object", "properties": {"choice": {"const": "left", "default": "left"}}, "required": ["choice"]},
                {"type": "object", "properties": {"choice": {"const": "right", "default": "right"}}, "required": ["choice"]}
            ]}},
            "fuzz.Item": {"schema": {"type": "object", "properties": {
                "id": {"type": "integer", "minimum": 0}, "count": {"type": "integer", "default": 1}, "metadata": {"type": "object"}
            }, "required": ["id"]}},
            "fuzz.Count": {"schema": {"type": "integer", "minimum": 0}},
            "fuzz.Text": {"schema": {"type": "string"}},
            "fuzz.Flag": {"schema": {"type": "boolean"}},
            "fuzz.List": {"schema": {"type": "array", "items": {}, "uniqueItems": true}}
        }, "contracts": {"fuzz.Provider": {"type": "object"}}});
        let provider = json!({"types": {"fuzz.NativeProvider": {
            "schema": {"type": "object", "properties": {"enabled": {
                "type": "boolean", "default": true,
                "x-daml-default-tags": [{"path": "", "tag": "fuzz.Flag"}]
            }}, "additionalProperties": true},
            "implements": ["fuzz.Provider"]
        }}});
        let known_tags = [&config, &provider]
            .into_iter()
            .flat_map(|module| module["types"].as_object().unwrap().keys().cloned())
            .collect();
        let config = SchemaModule::from_value(config).expect("fixed native module");
        let provider = SchemaModule::from_value(provider).expect("fixed independent provider");
        let schemas = Schemas::compose([config, provider]).expect("fixed native composition");
        let fixture = NativeFixture {
            schemas,
            known_tags,
        };
        // Fixed fixture checks run once, not for every mutated input.
        for document in fixed_documents() {
            let expected = document.root().tag.is_none().then_some("fuzz.Config");
            let result = fixture
                .schemas
                .resolve_with_limits(document, expected, RESOLUTION_LIMITS)
                .expect("fixed fixture resolves");
            check_resolved(result.root(), &fixture.known_tags, document.source());
        }
        let error = fixture
            .schemas
            .resolve_with_limits(ambiguous_document(), None, RESOLUTION_LIMITS)
            .expect_err("distinct valid default completions must remain ambiguous");
        assert_eq!(error.kind, ResolveErrorKind::AmbiguousDefault);
        assert!(matches!(error.origin, ErrorOrigin::Default(_)));
        fixture
    });
    &FIXTURE
}

fn fixed_documents() -> &'static [Document] {
    static DOCUMENTS: LazyLock<Vec<Document>> = LazyLock::new(|| {
        [
        "!fuzz.Config { choice: \"selected\"\n adapter: !fuzz.NativeProvider { note: null } }",
        "{ name: \"plain config\"\n count: 2\n items: [1\n3]\n choice: \"selected\"\n adapter: !fuzz.NativeProvider }",
        "!fuzz.Config { name: null\n count: 0\n items: []\n choice: \"selected\"\n adapter: !fuzz.NativeProvider { enabled: true } }",
        "!fuzz.Item { id: 42\n metadata: { label: \"wide\" } }",
        "!fuzz.List [!fuzz.Count 1\n!fuzz.Count 2]",
    ].into_iter().map(|source| parse_with_limits(source, PARSE_LIMITS).expect("fixed document")).collect()
    });
    &DOCUMENTS
}

fn ambiguous_document() -> &'static Document {
    static DOCUMENT: LazyLock<Document> = LazyLock::new(|| {
        parse_with_limits("!fuzz.Ambiguous {}", PARSE_LIMITS).expect("fixed ambiguity document")
    });
    &DOCUMENT
}

fn check_resolved(node: &ResolvedNode<'_>, known_tags: &HashSet<String>, source: &str) -> usize {
    if let Some(tag) = node.tag() {
        assert!(known_tags.contains(tag));
    }
    if let Origin::Input(span) = node.origin() {
        check_span(source, span);
    }
    let children = match node.value() {
        ResolvedValue::Sequence(items) => items
            .iter()
            .map(|item| check_resolved(item, known_tags, source))
            .sum(),
        ResolvedValue::Mapping(fields) => fields
            .values()
            .map(|item| check_resolved(item, known_tags, source))
            .sum(),
        _ => 0usize,
    };
    1 + children
}

fn check_error_origin(origin: ErrorOrigin, source: &str) {
    if let ErrorOrigin::Input(span) = origin {
        check_span(source, span);
    }
}

pub fn exercise_resolve(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    if source.len() > PARSE_LIMITS.max_input_bytes {
        return;
    }
    let Ok(document) = parse_with_limits(source, PARSE_LIMITS) else {
        return;
    };
    let fixture = native_fixture();
    for expected in [None]
        .into_iter()
        .chain(document.root().tag.is_none().then_some(Some("fuzz.Config")))
    {
        match fixture
            .schemas
            .resolve_with_limits(&document, expected, RESOLUTION_LIMITS)
        {
            Ok(resolved) => {
                assert_eq!(resolved.source(), source);
                assert!(
                    check_resolved(resolved.root(), &fixture.known_tags, source)
                        <= RESOLUTION_LIMITS.max_nodes
                );
            }
            Err(error) => check_error_origin(error.origin, source),
        }
    }
}

pub fn exercise_schema(data: &[u8]) {
    let Some((&mode, source)) = data.split_first() else {
        return;
    };
    let Ok(source) = std::str::from_utf8(source) else {
        return;
    };
    if source.len() > SCHEMA_INPUT_LIMIT {
        return;
    }
    let module = match mode {
        b'J' => SchemaModule::from_json(source),
        b'D' => SchemaModule::from_daml(source),
        _ => return,
    };
    let Ok(module) = module else { return };
    let Ok(schemas) = Schemas::compose([module]) else {
        return;
    };
    static PROBES: LazyLock<Vec<Document>> = LazyLock::new(|| {
        [
            "!fuzz.Config {}",
            "!fuzz.Config { trigger: null }",
            "!fuzz.Config []",
            "!fuzz.Config [1e400\n2e400]",
            "!fuzz.Config 1e400",
            "!fuzz.Config \"é\"",
            "!fuzz.Config null",
            "{}",
        ]
        .into_iter()
        .map(|source| parse_with_limits(source, PARSE_LIMITS).expect("fixed schema probe"))
        .collect()
    });
    for document in &*PROBES {
        let expected = document.root().tag.is_none().then_some("fuzz.Config");
        if let Err(error) = schemas.resolve_with_limits(document, expected, RESOLUTION_LIMITS) {
            check_error_origin(error.origin, document.source());
        }
    }
}
