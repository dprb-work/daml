//! Snapshot-relative source-edit acceptance checks through the public library API.

use daml::{
    Bundle, Document, ErrorKind, FormatOptions, Limits, SchemaModule, Schemas, SemanticErrorKind,
    SemanticNode, SemanticTag, SemanticValue, SourceEdit, SourceEditError, SourceEditLimits, Span,
    parse,
};
use serde_json::json;

fn semantic(source: &str) -> SemanticNode {
    SemanticNode::from_document(&parse(source).unwrap())
}

fn apply(document: &Document, edits: &[SourceEdit<'_>]) -> Document {
    let patch = document
        .prepare_source_edits(edits, FormatOptions::default(), SourceEditLimits::default())
        .unwrap();
    // Independently reconstruct the public change regions. Every untouched byte
    // must remain exact; this does not assume an ordering for changes().
    let mut changes: Vec<_> = patch.changes().iter().collect();
    changes.sort_by_key(|change| change.span.start);
    let mut expected = String::new();
    let mut cursor = 0;
    for change in changes {
        assert!(cursor <= change.span.start);
        assert!(change.span.start <= change.span.end);
        assert!(document.source().is_char_boundary(change.span.start));
        assert!(document.source().is_char_boundary(change.span.end));
        expected.push_str(&document.source()[cursor..change.span.start]);
        expected.push_str(&change.replacement);
        cursor = change.span.end;
    }
    expected.push_str(&document.source()[cursor..]);
    let result = patch.apply(document.source()).unwrap();
    assert_eq!(result.source(), expected);
    result
}

fn assert_value(document: &Document, expected: &str) {
    assert_eq!(SemanticNode::from_document(document), semantic(expected));
}

fn bounded(
    document: &Document,
    edits: &[SourceEdit<'_>],
    limits: SourceEditLimits,
) -> Result<Document, SourceEditError> {
    document
        .prepare_source_edits(edits, FormatOptions::default(), limits)?
        .apply(document.source())
}

#[test]
fn tutorial_endpoint_replacement_is_one_exact_region_without_completed_defaults() {
    let source = "# before\r\n!app.Service {\r\n name: \"demo\"\r\n adapter: !providers.Http {\r\n  # keep endpoint explanation\r\n  endpoint: \"https://old.example\" # keep trailing\r\n  timeout: 3.0e1\r\n }\r\n}";
    let document = parse(source).unwrap();
    let schemas = Bundle::open(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/tutorial"))
        .unwrap()
        .compile(&document, None)
        .unwrap();
    let value = semantic("\"https://new.example\"");
    let patch = schemas
        .prepare_source_edits(
            &document,
            None,
            &[SourceEdit::Replace {
                path: "/adapter/endpoint",
                value: &value,
            }],
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .unwrap();
    let start = source.find("\"https://old.example\"").unwrap();
    assert_eq!(patch.changes().len(), 1);
    assert_eq!(
        patch.changes()[0].span,
        Span {
            start,
            end: start + "\"https://old.example\"".len()
        }
    );
    let edited = patch.apply(source).unwrap();
    assert_eq!(
        edited.source(),
        source.replacen("\"https://old.example\"", "\"https://new.example\"", 1)
    );
    assert!(edited.get(edited.root(), "retries").is_none());
    assert_eq!(schemas.resolve(&edited).unwrap().to_json()["retries"], 3);
    assert_eq!(document.source(), source);
}

#[test]
fn replacement_owns_tag_and_internal_comments_but_not_adjacent_comments() {
    let source = "# before root\n{\n # before selected\n selected: !app.Old {\n  # internal disappears\n  child: 1\n } # after selected\n keep: 2.00e0\n}\n# after root";
    let document = parse(source).unwrap();
    let value = semantic("!app.New [true]");
    let edited = apply(
        &document,
        &[SourceEdit::Replace {
            path: "/selected",
            value: &value,
        }],
    );
    assert_eq!(
        edited.source(),
        source.replacen(
            "!app.Old {\n  # internal disappears\n  child: 1\n }",
            "!app.New [true]",
            1
        )
    );
    assert_eq!(document.source(), source);
}

#[test]
fn map_and_array_removal_leave_external_comments_and_all_trivia() {
    for (source, path, removed, expected_value) in [
        (
            "{\n # before\n gone: # between key and value\n !app.Old {\n  # internal\n  x: 1\n } # after\n keep: 2\n}",
            "/gone",
            "gone: # between key and value\n !app.Old {\n  # internal\n  x: 1\n }",
            "{keep: 2}",
        ),
        (
            "[\n # before\n !app.Old {\n  # internal\n  x: 1\n } # after\n 2\n]",
            "/0",
            "!app.Old {\n  # internal\n  x: 1\n }",
            "[2]",
        ),
    ] {
        let document = parse(source).unwrap();
        let edited = apply(&document, &[SourceEdit::Remove { path }]);
        assert_eq!(edited.source(), source.replacen(removed, "", 1));
        assert_value(&edited, expected_value);
    }
    for (source, path, expected) in [
        (
            "{first:1\nmiddle:2\nlast:3}",
            "/first",
            "{middle:2\nlast:3}",
        ),
        (
            "{first:1\nmiddle:2\nlast:3}",
            "/middle",
            "{first:1\nlast:3}",
        ),
        (
            "{first:1\nmiddle:2\nlast:3}",
            "/last",
            "{first:1\nmiddle:2}",
        ),
        ("{only:1}", "/only", "{}"),
        ("[1\n2\n3]", "/0", "[2\n3]"),
        ("[1\n2\n3]", "/1", "[1\n3]"),
        ("[1\n2\n3]", "/2", "[1\n2]"),
        ("[1]", "/0", "[]"),
    ] {
        assert_value(
            &apply(&parse(source).unwrap(), &[SourceEdit::Remove { path }]),
            expected,
        );
    }
}

#[test]
fn all_batch_array_paths_use_original_indices_not_shifted_or_sequential_state() {
    let document =
        parse("{items:[{name:\"zero\"}\n{name:\"one\"}\n{name:\"two\"}]\nkeep:4}").unwrap();
    let changed = semantic("\"changed one\"");
    let appended = semantic("{name:\"appended\"}");
    let edited = apply(
        &document,
        &[
            SourceEdit::Remove { path: "/items/0" },
            SourceEdit::Replace {
                path: "/items/1/name",
                value: &changed,
            },
            SourceEdit::Insert {
                path: "/items/3",
                value: &appended,
            },
        ],
    );
    assert_value(
        &edited,
        "{items:[{name:\"changed one\"}\n{name:\"two\"}\n{name:\"appended\"}]\nkeep:4}",
    );

    // Reverse operation order cannot change which original node is selected.
    let reversed = apply(
        &document,
        &[
            SourceEdit::Insert {
                path: "/items/3",
                value: &appended,
            },
            SourceEdit::Replace {
                path: "/items/1/name",
                value: &changed,
            },
            SourceEdit::Remove { path: "/items/0" },
        ],
    );
    assert_eq!(edited.source(), reversed.source());
}

#[test]
fn insertion_appends_map_fields_without_reordering_and_uses_original_array_boundaries() {
    let value = semantic("9");
    let document = parse("{z:1\n# leave here\na:2 # trailing\n}").unwrap();
    let edited = apply(
        &document,
        &[SourceEdit::Insert {
            path: "/middle",
            value: &value,
        }],
    );
    assert_value(&edited, "{z:1\na:2\nmiddle:9}");
    let z = edited.source().find("z:1").unwrap();
    let a = edited.source().find("a:2 # trailing").unwrap();
    let inserted = edited.source().find("middle:").unwrap();
    assert!(z < a && a < inserted);
    assert!(edited.source().contains("# leave here"));
    for (path, expected) in [
        ("/0", "[9\n1\n2]"),
        ("/1", "[1\n9\n2]"),
        ("/2", "[1\n2\n9]"),
    ] {
        assert_value(
            &apply(
                &parse("[1\n2]").unwrap(),
                &[SourceEdit::Insert {
                    path,
                    value: &value,
                }],
            ),
            expected,
        );
    }
    let edited = apply(
        &parse("[1\n2]").unwrap(),
        &[
            SourceEdit::Insert {
                path: "/0",
                value: &value,
            },
            SourceEdit::Insert {
                path: "/2",
                value: &value,
            },
        ],
    );
    assert_value(&edited, "[9\n1\n2\n9]");
}

#[test]
fn empty_containers_and_tagged_implicit_maps_admit_insertions() {
    let value = semantic("true");
    for (source, path, expected) in [
        ("{}", "/enabled", "{enabled:true}"),
        ("[]", "/0", "[true]"),
        ("!app.Empty", "/enabled", "!app.Empty {enabled:true}"),
        (
            "{nested: !app.Empty # keep\n}",
            "/nested/enabled",
            "{nested: !app.Empty {enabled:true}}",
        ),
        ("{nested: {}}", "/nested/enabled", "{nested:{enabled:true}}"),
    ] {
        let edited = apply(
            &parse(source).unwrap(),
            &[SourceEdit::Insert {
                path,
                value: &value,
            }],
        );
        assert_value(&edited, expected);
        if source.contains("# keep") {
            assert!(edited.source().contains("# keep\n"));
        }
    }
}

#[test]
fn pointer_tokens_are_literal_keys_and_escapes_are_validated_over_the_whole_pointer() {
    let source = "{a/b:1\na.b:2\n00:3\na:{b:4}}";
    let value = semantic("9");
    let document = parse(source).unwrap();
    let edited = apply(
        &document,
        &[
            SourceEdit::Replace {
                path: "/a~1b",
                value: &value,
            },
            SourceEdit::Replace {
                path: "/a.b",
                value: &value,
            },
            SourceEdit::Replace {
                path: "/00",
                value: &value,
            },
        ],
    );
    assert_value(&edited, "{a/b:9\na.b:9\n00:9\na:{b:4}}");
    // Tilde is not in the DAML key alphabet, but ~0 is a valid pointer
    // escape. It must be decoded, not rejected as malformed pointer syntax.
    for path in ["/~0", "/a~01b"] {
        let error = document
            .prepare_source_edits(
                &[SourceEdit::Remove { path }],
                FormatOptions::default(),
                SourceEditLimits::default(),
            )
            .err()
            .unwrap();
        assert!(matches!(error, SourceEditError::PathNotFound(_)));
    }
    for path in ["a/b", "/a~", "/a~2b", "/absent/~2", "/absent/~"] {
        let error = document
            .prepare_source_edits(
                &[SourceEdit::Remove { path }],
                FormatOptions::default(),
                SourceEditLimits::default(),
            )
            .err()
            .unwrap();
        assert!(
            matches!(error, SourceEditError::InvalidPointer(_)),
            "{path}: {error}"
        );
    }
}

#[test]
fn root_replacement_preserves_external_bytes_and_invalid_targets_do_not_create_parents() {
    let source = "\u{feff}# before\r\n!app.Old {old:1} # after";
    let document = parse(source).unwrap();
    let value = semantic("[true]");
    let edited = apply(
        &document,
        &[SourceEdit::Replace {
            path: "",
            value: &value,
        }],
    );
    assert_eq!(edited.source(), "\u{feff}# before\r\n[true] # after");
    for edit in [
        SourceEdit::Remove { path: "" },
        SourceEdit::Insert {
            path: "",
            value: &value,
        },
    ] {
        assert!(
            document
                .prepare_source_edits(
                    &[edit],
                    FormatOptions::default(),
                    SourceEditLimits::default()
                )
                .is_err()
        );
    }
    let document = parse("{array:[1\n2]\nscalar:1\nexisting:true}").unwrap();
    for edit in [
        SourceEdit::Insert {
            path: "/existing",
            value: &value,
        },
        SourceEdit::Insert {
            path: "/missing/child",
            value: &value,
        },
        SourceEdit::Insert {
            path: "/scalar/child",
            value: &value,
        },
        SourceEdit::Replace {
            path: "/absent",
            value: &value,
        },
        SourceEdit::Remove { path: "/array/2" },
        SourceEdit::Replace {
            path: "/array/2",
            value: &value,
        },
        SourceEdit::Insert {
            path: "/array/3",
            value: &value,
        },
    ] {
        assert!(
            document
                .prepare_source_edits(
                    &[edit],
                    FormatOptions::default(),
                    SourceEditLimits::default()
                )
                .is_err()
        );
    }
    for index in [
        "-",
        "+0",
        "-1",
        "00",
        "01",
        "1.0",
        " 0",
        "999999999999999999999999999999999999999999999999999999999",
    ] {
        let path = format!("/array/{index}");
        for edit in [
            SourceEdit::Insert {
                path: &path,
                value: &value,
            },
            SourceEdit::Remove { path: &path },
            SourceEdit::Replace {
                path: &path,
                value: &value,
            },
        ] {
            assert!(
                document
                    .prepare_source_edits(
                        &[edit],
                        FormatOptions::default(),
                        SourceEditLimits::default()
                    )
                    .is_err(),
                "accepted index {index}"
            );
        }
    }
}

#[test]
fn logical_and_byte_boundary_conflicts_report_original_operation_indices() {
    let document = parse("{items:[{name:\"one\"}\n{name:\"two\"}]\nmap:{a:1}}").unwrap();
    let value = semantic("7");
    let cases = [
        [
            SourceEdit::Remove { path: "/map/a" },
            SourceEdit::Replace {
                path: "/map/a",
                value: &value,
            },
        ],
        [
            SourceEdit::Remove { path: "/items/0" },
            SourceEdit::Replace {
                path: "/items/0/name",
                value: &value,
            },
        ],
        [
            SourceEdit::Replace {
                path: "",
                value: &value,
            },
            SourceEdit::Remove { path: "/map/a" },
        ],
        [
            SourceEdit::Insert {
                path: "/items/1",
                value: &value,
            },
            SourceEdit::Insert {
                path: "/items/1",
                value: &value,
            },
        ],
        [
            SourceEdit::Insert {
                path: "/map/b",
                value: &value,
            },
            SourceEdit::Insert {
                path: "/map/c",
                value: &value,
            },
        ],
    ];
    for edits in cases {
        let error = document
            .prepare_source_edits(
                &edits,
                FormatOptions::default(),
                SourceEditLimits::default(),
            )
            .err()
            .unwrap();
        assert!(
            matches!(
                error,
                SourceEditError::Conflict {
                    first: 0,
                    second: 1
                }
            ),
            "{error}"
        );
    }
    let error = document
        .prepare_source_edits(
            &[
                SourceEdit::Remove { path: "/map/a" },
                SourceEdit::Replace {
                    path: "/items/1/name",
                    value: &value,
                },
                SourceEdit::Remove { path: "/items/1" },
            ],
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .err()
        .unwrap();
    assert!(matches!(
        error,
        SourceEditError::Conflict {
            first: 1,
            second: 2
        }
    ));
}

#[test]
fn stale_source_checks_all_bytes_even_for_empty_batches() {
    let source = "{value:1} # original";
    let document = parse(source).unwrap();
    let value = semantic("2");
    for edits in [
        vec![],
        vec![SourceEdit::Replace {
            path: "/value",
            value: &value,
        }],
    ] {
        let patch = document
            .prepare_source_edits(
                &edits,
                FormatOptions::default(),
                SourceEditLimits::default(),
            )
            .unwrap();
        for stale in [
            "{value:1} # changed",
            "{value:1} # original\n",
            "{value:2} # original",
        ] {
            assert!(matches!(
                patch.apply(stale),
                Err(SourceEditError::StaleSource)
            ));
        }
        let same_bytes = source.to_owned();
        let edited = patch.apply(&same_bytes).unwrap();
        if edits.is_empty() {
            assert!(patch.changes().is_empty());
            assert_eq!(edited.source(), source);
        }
    }
    assert_eq!(document.source(), source);
}

#[test]
fn utf8_byte_regions_bom_and_mixed_line_endings_are_preserved() {
    let source = "\u{feff}# café\r\n{\r\n 名: \"古い\" # 雪\n keep: 1.00\r}";
    let document = parse(source).unwrap();
    let value = semantic("\"新しい🌍\"");
    let patch = document
        .prepare_source_edits(
            &[SourceEdit::Replace {
                path: "/名",
                value: &value,
            }],
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .unwrap();
    let start = source.find("\"古い\"").unwrap();
    assert_eq!(patch.changes().len(), 1);
    assert_eq!(
        patch.changes()[0].span,
        Span {
            start,
            end: start + "\"古い\"".len()
        }
    );
    let edited = patch.apply(source).unwrap();
    assert_eq!(
        edited.source(),
        source.replacen("\"古い\"", "\"新しい🌍\"", 1)
    );
}

#[test]
fn generated_structural_newlines_use_parent_style_without_changing_multiline_data() {
    let value = semantic("{first: \"line one\\nline two\\r\\nline three\"\nsecond: [1\n2]}");
    for eol in ["\r\n", "\r", "\n"] {
        let source = format!("{{{eol} keep: 1\n}}");
        let document = parse(&source).unwrap();
        let patch = document
            .prepare_source_edits(
                &[SourceEdit::Insert {
                    path: "/new",
                    value: &value,
                }],
                FormatOptions { line_width: 20 },
                SourceEditLimits::default(),
            )
            .unwrap();
        let edited = patch.apply(&source).unwrap();
        let actual = SemanticNode::from_document(&edited);
        let SemanticValue::Mapping(fields) = actual.value else {
            panic!("map")
        };
        assert_eq!(fields["new"], value);
        assert!(edited.source().contains(" keep: 1\n"));
        for change in patch.changes() {
            let structural = change.replacement.replace(eol, "");
            assert!(
                !structural.contains(['\r', '\n']),
                "wrong generated EOL: {:?}",
                change.replacement
            );
        }
    }
    for block in [
        "\"\"\"\n  first\n  second\n  \"\"\"",
        "'''\r\n  first\r\n  second\r\n  '''",
    ] {
        let value = semantic(block);
        let document = parse("{value:null\r\nkeep:1}").unwrap();
        let edited = apply(
            &document,
            &[SourceEdit::Replace {
                path: "/value",
                value: &value,
            }],
        );
        let SemanticValue::Mapping(fields) = SemanticNode::from_document(&edited).value else {
            panic!("map")
        };
        assert_eq!(fields["value"], value);
    }
}

#[test]
fn exact_numbers_temporals_cron_and_unqualified_assertions_roundtrip_new_regions() {
    let document = parse("{value:null\nunchanged:9.00e2}").unwrap();
    for source in [
        "9007199254740993123456789.123456789",
        "-0.000000000000000000123456789",
        "1e1000",
        "!Date \"2024-01-02\"",
        "!Time \"09:30:00.123456789123\"",
        "!DateTime \"2026-10-29T09:30:00.123456789Z\"",
        "!Cron \"0 0 9 * * *\"",
        "!app.Asserted {x:1}",
    ] {
        let value = semantic(source);
        let edited = apply(
            &document,
            &[SourceEdit::Replace {
                path: "/value",
                value: &value,
            }],
        );
        let SemanticValue::Mapping(fields) = SemanticNode::from_document(&edited).value else {
            panic!("map")
        };
        assert_eq!(fields["value"], value, "changed supplied value {source}");
        assert!(edited.source().contains("unchanged:9.00e2"));
    }
}

#[test]
fn unavailable_binding_is_refused_instead_of_stripped_or_rebound_by_name() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({
        "types": {"Root": {"schema": {"type": "string"}}}
    }))
    .unwrap()])
    .unwrap();
    let value = SemanticNode {
        tag: Some(SemanticTag {
            name: "Root".into(),
            binding: Some("urn:unavailable".into()),
        }),
        value: SemanticValue::String("same shape".into()),
    };
    let document = parse("\"old\"").unwrap();
    let edits = [SourceEdit::Replace {
        path: "",
        value: &value,
    }];
    for error in [
        document
            .prepare_source_edits(
                &edits,
                FormatOptions::default(),
                SourceEditLimits::default(),
            )
            .err()
            .unwrap(),
        schemas
            .prepare_source_edits(
                &document,
                Some("Root"),
                &edits,
                FormatOptions::default(),
                SourceEditLimits::default(),
            )
            .err()
            .unwrap(),
    ] {
        assert!(
            matches!(&error, SourceEditError::Semantic(cause) if cause.kind == SemanticErrorKind::UnrepresentableTag)
        );
    }
    assert_eq!(document.source(), "\"old\"");
}

#[test]
fn contextual_application_validates_final_value_and_empty_batches_without_writing_defaults() {
    let schemas = Schemas::compose([SchemaModule::from_value(json!({"types": {"app.Config": {"schema": {
        "type": "object", "properties": {"endpoint": {"type": "string"}, "timeout": {"type": "integer", "default": 30}},
        "required": ["endpoint", "timeout"], "additionalProperties": false
    }}}})).unwrap()]).unwrap();
    let original = "{endpoint:7}";
    let document = parse(original).unwrap();
    let repair = semantic("\"fixed\"");
    let patch = schemas
        .prepare_source_edits(
            &document,
            Some("app.Config"),
            &[SourceEdit::Replace {
                path: "/endpoint",
                value: &repair,
            }],
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .unwrap();
    let edited = patch.apply(original).unwrap();
    assert_eq!(edited.source(), "{endpoint:\"fixed\"}");
    assert!(edited.get(edited.root(), "timeout").is_none());
    let empty = schemas
        .prepare_source_edits(
            &document,
            Some("app.Config"),
            &[],
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .unwrap();
    assert!(matches!(
        empty.apply(original),
        Err(SourceEditError::Resolve(_))
    ));
    let valid = parse("{endpoint:\"old\"}").unwrap();
    let invalid = semantic("false");
    let patch = schemas
        .prepare_source_edits(
            &valid,
            Some("app.Config"),
            &[SourceEdit::Replace {
                path: "/endpoint",
                value: &invalid,
            }],
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .unwrap();
    assert!(matches!(
        patch.apply(valid.source()),
        Err(SourceEditError::Resolve(_))
    ));
    assert_eq!(document.source(), original);
}

#[test]
fn explicit_operation_and_output_limits_apply_to_the_whole_batch() {
    let document = parse("{a:1\nb:2}").unwrap();
    let value = semantic("123");
    let edits = [
        SourceEdit::Replace {
            path: "/a",
            value: &value,
        },
        SourceEdit::Replace {
            path: "/b",
            value: &value,
        },
    ];
    let sufficient = || SourceEditLimits {
        max_operations: 2,
        max_work: 4096,
        max_output_bytes: 128,
        parse: Limits::default(),
    };
    let edited = bounded(&document, &edits, sufficient()).unwrap();
    assert!(matches!(
        bounded(
            &document,
            &edits,
            SourceEditLimits {
                max_operations: 1,
                ..sufficient()
            }
        ),
        Err(SourceEditError::LimitExceeded)
    ));
    let exact_output = SourceEditLimits {
        max_output_bytes: edited.source().len(),
        ..sufficient()
    };
    assert_eq!(
        bounded(&document, &edits, exact_output).unwrap().source(),
        edited.source()
    );
    assert!(matches!(
        bounded(
            &document,
            &edits,
            SourceEditLimits {
                max_output_bytes: edited.source().len() - 1,
                ..sufficient()
            }
        ),
        Err(SourceEditError::LimitExceeded)
    ));
    // Empty batches do not bypass output admission.
    assert!(matches!(
        bounded(
            &document,
            &[],
            SourceEditLimits {
                max_output_bytes: document.source().len() - 1,
                ..sufficient()
            }
        ),
        Err(SourceEditError::LimitExceeded)
    ));
    assert_eq!(document.source(), "{a:1\nb:2}");
}

#[test]
fn aggregate_work_budget_cannot_reset_for_each_operation_or_ignore_untouched_source() {
    let document =
        parse("{a:null\nb:null\nc:null\nd:null\ne:null\nf:null\ng:null\nh:null}").unwrap();
    let value = SemanticNode {
        tag: None,
        value: SemanticValue::String("x".repeat(1024)),
    };
    let edits: Vec<_> = ["/a", "/b", "/c", "/d", "/e", "/f", "/g", "/h"]
        .into_iter()
        .map(|path| SourceEdit::Replace {
            path,
            value: &value,
        })
        .collect();
    // Each supplied payload fits; their combined payload bytes alone exceed
    // this explicit budget, even before pointer/render/source work is charged.
    let limits = || SourceEditLimits {
        max_operations: 8,
        max_work: 8191,
        max_output_bytes: 16384,
        parse: Limits::default(),
    };
    for edit in &edits {
        assert!(bounded(&document, std::slice::from_ref(edit), limits()).is_ok());
    }
    assert!(matches!(
        bounded(&document, &edits, limits()),
        Err(SourceEditError::LimitExceeded)
    ));
    let source = format!("# {}\n{{}}", "x".repeat(512));
    let large = parse(&source).unwrap();
    assert!(matches!(
        bounded(
            &large,
            &[],
            SourceEditLimits {
                max_work: source.len() - 1,
                ..limits()
            }
        ),
        Err(SourceEditError::LimitExceeded)
    ));
}

#[test]
fn edited_document_is_reparsed_under_declared_node_and_scalar_bounds() {
    let document = parse("{}").unwrap();
    let value = semantic("\"new string\"");
    let edits = [SourceEdit::Insert {
        path: "/new",
        value: &value,
    }];
    for parse_limits in [
        Limits {
            max_nodes: 1,
            ..Limits::default()
        },
        Limits {
            max_scalar_bytes: 4,
            ..Limits::default()
        },
    ] {
        let error = bounded(
            &document,
            &edits,
            SourceEditLimits {
                parse: parse_limits,
                ..SourceEditLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SourceEditError::LimitExceeded
                | SourceEditError::Parse(daml::ParseError {
                    kind: ErrorKind::LimitExceeded,
                    ..
                })
        ));
    }
    assert_eq!(document.source(), "{}");
}

#[test]
fn contextual_rendering_keeps_representable_native_binding_identity() {
    let bundle = Bundle::from_sources(
        ".",
        [
            ("daml.bundle".to_owned(), "{}".to_owned()),
            (
                "Host.daml".to_owned(),
                "!Object {payload: !self.Text}".to_owned(),
            ),
            ("Text.daml".to_owned(), "!String".to_owned()),
        ],
    )
    .unwrap();
    let document = parse("{payload:\"old\"}").unwrap();
    let schemas = bundle
        .compile_with_types(&document, Some("self.Host"), ["self.Text"])
        .unwrap();
    let supplied = parse("!self.Text \"new\"").unwrap();
    let value = SemanticNode::from_resolved(schemas.resolve(&supplied).unwrap().root());
    let binding = value.tag.as_ref().unwrap().binding.as_deref().unwrap();
    let edits = [SourceEdit::Replace {
        path: "/payload",
        value: &value,
    }];
    assert!(matches!(
        document.prepare_source_edits(
            &edits,
            FormatOptions::default(),
            SourceEditLimits::default()
        ),
        Err(SourceEditError::Semantic(_))
    ));
    let patch = schemas
        .prepare_source_edits(
            &document,
            Some("self.Host"),
            &edits,
            FormatOptions::default(),
            SourceEditLimits::default(),
        )
        .unwrap();
    let edited = patch.apply(document.source()).unwrap();
    let resolved = schemas.resolve_as(&edited, "self.Host").unwrap();
    assert_eq!(
        resolved.root().get("payload").unwrap().tag_binding(),
        Some(binding)
    );
    assert_eq!(resolved.to_json(), json!({"payload": "new"}));
}
