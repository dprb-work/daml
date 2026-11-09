//! Exact-asset consumer checks, independent of the grammar implementation.
//! Complete examples follow the existing DAML parser fixtures; deliberate fragments
//! test display recovery, not language acceptance or schema validation.

use std::ops::Range;

use syntect::easy::{HighlightLines, ScopeRangeIterator};
use syntect::highlighting::{Highlighter, ThemeSet};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxDefinition, SyntaxSet};

const GRAMMAR: &str = include_str!("../../../syntect/daml.sublime-syntax");
const PROPERTY: &str = "entity.name.label";
const TYPE: &str = "entity.name.type";
const STRING: &str = "string.quoted";
const REGEX: &str = "string.regexp";
const ESCAPE: &str = "constant.character.escape";
const COMMENT: &str = "comment.line";
const NUMBER: &str = "constant.numeric";
const OPERATOR: &str = "keyword.operator";

fn syntax_set() -> SyntaxSet {
    let definition = SyntaxDefinition::load_from_str(GRAMMAR, true, Some("DAML"))
        .expect("the exact published DAML asset loads through Syntect's YAML loader");
    let mut builder = syntect::parsing::SyntaxSetBuilder::new();
    builder.add(definition);
    let set = builder.build();
    assert!(set.find_unlinked_contexts().is_empty());
    set
}

// Unlike str::lines and Syntect's LF-based LinesWithEndings, this consumer
// explicitly supports DAML's CR-only physical lines as well as LF and CRLF.
fn physical_lines(source: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start == source.len() {
            return None;
        }
        let end = source[start..]
            .bytes()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(source.len(), |relative| {
                let newline = start + relative;
                newline
                    + if source.as_bytes()[newline..].starts_with(b"\r\n") {
                        2
                    } else {
                        1
                    }
            });
        let line = &source[start..end];
        start = end;
        Some(line)
    })
}

#[derive(Debug, PartialEq, Eq)]
struct ScopedRange {
    bytes: Range<usize>,
    scopes: Vec<Scope>,
}

#[derive(Debug, PartialEq, Eq)]
struct Trace {
    ranges: Vec<ScopedRange>,
    parser: ParseState,
    stack: ScopeStack,
}

fn feed(set: &SyntaxSet, line: &str, offset: usize, trace: &mut Trace) {
    let operations = trace
        .parser
        .parse_line(line, set)
        .expect("Syntect parses line");
    for (range, operation) in ScopeRangeIterator::new(&operations, line) {
        assert!(line.is_char_boundary(range.start));
        assert!(line.is_char_boundary(range.end));
        trace.stack.apply(operation).expect("valid scope operation");
        if !range.is_empty() {
            trace.ranges.push(ScopedRange {
                bytes: offset + range.start..offset + range.end,
                scopes: trace.stack.scopes.clone(),
            });
        }
    }
}

fn observe(set: &SyntaxSet, source: &str, pause: Option<usize>) -> Trace {
    let syntax = set.find_syntax_by_token("daml").unwrap();
    let mut trace = Trace {
        ranges: Vec::new(),
        parser: ParseState::new(syntax),
        stack: ScopeStack::new(),
    };
    let mut offset = 0;
    for (index, line) in physical_lines(source).enumerate() {
        if pause == Some(index) {
            // Simulate resuming from a consumer's independent cached checkpoint.
            let checkpoint = (trace.parser.clone(), trace.stack.clone());
            trace.parser = checkpoint.0;
            trace.stack = checkpoint.1;
        }
        feed(set, line, offset, &mut trace);
        offset += line.len();
    }
    assert_eq!(offset, source.len());
    trace
}

fn has_scope(scopes: &[Scope], prefix: &str) -> bool {
    let wanted = Scope::new(prefix).unwrap();
    scopes.iter().any(|scope| wanted.is_prefix_of(*scope))
}

fn assert_role(source: &str, trace: &Trace, needle: &str, expected: &str, rejected: &[&str]) {
    let start = source.find(needle).expect("probe exists in source");
    assert_role_at(
        source,
        trace,
        start..start + needle.len(),
        expected,
        rejected,
    );
}

fn assert_role_at(
    source: &str,
    trace: &Trace,
    bytes: Range<usize>,
    expected: &str,
    rejected: &[&str],
) {
    assert!(!bytes.is_empty());
    for (relative, _) in source[bytes.clone()].char_indices() {
        let position = bytes.start + relative;
        let scopes = &trace
            .ranges
            .iter()
            .find(|range| range.bytes.contains(&position))
            .expect("every source character has a scope range")
            .scopes;
        assert!(
            has_scope(scopes, "source.daml"),
            "missing root at {position}"
        );
        assert!(
            has_scope(scopes, expected),
            "expected {expected} at {position}: {scopes:?}"
        );
        for unwanted in rejected {
            assert!(
                !has_scope(scopes, unwanted),
                "unexpected {unwanted} at {position}: {scopes:?}"
            );
        }
    }
}

#[test]
fn exact_asset_loads_without_external_syntax_and_selects_by_consumer_metadata() {
    let set = syntax_set();
    assert_eq!(set.syntaxes().len(), 1);
    let scope = Scope::new("source.daml").unwrap();
    for syntax in [
        set.find_syntax_by_scope(scope),
        set.find_syntax_by_name("DAML"),
        set.find_syntax_by_token("daml"),
        set.find_syntax_by_extension("daml"),
        set.find_syntax_for_file("example.daml").unwrap(),
        set.find_syntax_for_file("daml.bundle").unwrap(),
    ] {
        let syntax = syntax.expect("consumer metadata selects DAML");
        assert_eq!(syntax.scope, scope);
        assert_eq!(syntax.name, "DAML");
    }
    assert!(set.find_syntax_by_extension("bundle").is_none());
}

#[test]
fn properties_outrank_keyword_numeric_and_dotted_type_lookalikes_in_fragments() {
    let set = syntax_set();
    // No root braces or selected schema are required by a display grammar.
    let source = "true: false\nfalse: true\nnull: null\nclosed: 1\nunset: 2\n200: 3\napp.Config: !app.Config\nname?: !String\né: 4\né: 5\n名前: 6\n";
    let trace = observe(&set, source, None);
    for key in [
        "true",
        "false",
        "null",
        "closed",
        "unset",
        "200",
        "app.Config",
        "name",
        "é",
        "é",
        "名前",
    ] {
        let suffix = if key == "name" { "?:" } else { ":" };
        let start = source.find(&format!("{key}{suffix}")).unwrap();
        assert_role_at(
            source,
            &trace,
            start..start + key.len(),
            PROPERTY,
            &["constant", "keyword", TYPE],
        );
    }
    assert_role(source, &trace, "?", OPERATOR, &[PROPERTY]);
    assert_role(source, &trace, "!app.Config", TYPE, &[PROPERTY]);
}

#[test]
fn numbers_literals_native_operators_and_collection_punctuation_have_standard_roles() {
    let set = syntax_set();
    let source = "{values: [-12.50e+3 1E-9 0 -0 true false null] tag: !app.network.Host}\nclosed !Object {primary?: !String secondary?: !String} & !Map {min: 1 max: 1}\n(!Int | !String) = 3\n!Regex \"a\" = unset # tail\n";
    let trace = observe(&set, source, None);
    for number in ["-12.50e+3", "1E-9", "0", "-0"] {
        assert_role(source, &trace, number, NUMBER, &[PROPERTY, STRING]);
    }
    for literal in ["true", "false", "null"] {
        assert_role(source, &trace, literal, "constant.language", &[PROPERTY]);
    }
    for keyword in ["closed", "unset"] {
        assert_role(source, &trace, keyword, "keyword.control", &[PROPERTY]);
    }
    for (offset, character) in source.char_indices() {
        if "{}[]()".contains(character) {
            assert_role_at(source, &trace, offset..offset + 1, "punctuation", &[STRING]);
        } else if "&|=?:".contains(character) {
            assert_role_at(source, &trace, offset..offset + 1, OPERATOR, &[PROPERTY]);
        }
    }
    assert_role(source, &trace, "!app.network.Host", TYPE, &[PROPERTY]);
    assert_role(source, &trace, "# tail", COMMENT, &[REGEX, STRING]);
    let end = source.find("-12.50e+3").unwrap() + "-12.50e+3".len();
    let following = trace
        .ranges
        .iter()
        .find(|range| range.bytes.contains(&end))
        .unwrap();
    assert!(!has_scope(&following.scopes, NUMBER));
}

#[test]
fn ordinary_quotes_keep_escapes_and_unicode_opaque_then_restore_comments() {
    let set = syntax_set();
    let source = concat!(
        "{name: \"😀 café ",
        r#"\"quoted\" \\ \u0041 # !Tag key: | & = ?"#,
        "\" # trailing\nnext: 12}\n",
    );
    let trace = observe(&set, source, None);
    for body in ["😀", "café", "quoted", "# !Tag key: | & = ?"] {
        assert_role(
            source,
            &trace,
            body,
            STRING,
            &[COMMENT, PROPERTY, TYPE, OPERATOR],
        );
    }
    for escaped in [r#"\""#, r"\\", r"\u0041"] {
        assert_role(source, &trace, escaped, ESCAPE, &[COMMENT]);
    }
    assert_role(source, &trace, "# trailing", COMMENT, &[STRING]);
    assert_role(source, &trace, "next", PROPERTY, &[STRING]);

    let slash = concat!(r#""ends with slash \\" # outside"#, "\n");
    let trace = observe(&set, slash, None);
    assert_role(slash, &trace, "# outside", COMMENT, &[STRING]);
}

#[test]
fn incomplete_ordinary_strings_do_not_swallow_following_physical_lines() {
    let set = syntax_set();
    for eol in ["\n", "\r\n", "\r"] {
        for tail in ["", "\\"] {
            let source = format!("broken: \"unfinished{tail}{eol}next: 12 # outside{eol}");
            let trace = observe(&set, &source, None);
            assert_role(&source, &trace, "unfinished", STRING, &[COMMENT]);
            assert_role(&source, &trace, "next", PROPERTY, &[STRING]);
            assert_role(&source, &trace, "12", NUMBER, &[STRING]);
            assert_role(&source, &trace, "# outside", COMMENT, &[STRING]);
        }
    }
}

#[test]
fn ordinary_triples_preserve_opaque_bodies_escaped_delimiters_and_context_exit() {
    let set = syntax_set();
    for delimiter in ["\"\"\"", "'''"] {
        for eol in ["\n", "\r\n", "\r"] {
            for slashes in [1, 3] {
                let source = format!(
                    "{{text: {delimiter}{eol}  # !Tag field: | &{eol}{eol}  a {}{delimiter} b{eol}  two \\\\ slashes{eol}  {delimiter} # trailing{eol}next: 12{eol}}}",
                    "\\".repeat(slashes)
                );
                let trace = observe(&set, &source, None);
                for body in ["# !Tag field: | &", " b", "two"] {
                    assert_role(
                        &source,
                        &trace,
                        body,
                        STRING,
                        &[COMMENT, PROPERTY, OPERATOR, TYPE],
                    );
                }
                assert_role(&source, &trace, "\\\\", ESCAPE, &[COMMENT]);
                assert_role(&source, &trace, "# trailing", COMMENT, &[STRING]);
                assert_role(&source, &trace, "next", PROPERTY, &[STRING]);
            }
        }
    }
}

#[test]
fn raw_regex_single_line_has_no_daml_escapes_and_quote_closes_after_backslash() {
    let set = syntax_set();
    let source = "!String {pattern: !Regex \"\\d+\\\\x# !Tag key: | &\" # trailing\n}\n";
    let trace = observe(&set, source, None);
    assert_role(source, &trace, "!Regex", TYPE, &[REGEX]);
    assert_role(
        source,
        &trace,
        "\\d+\\\\x# !Tag key: | &",
        REGEX,
        &[ESCAPE, COMMENT, TYPE, PROPERTY, OPERATOR],
    );
    assert_role(source, &trace, "# trailing", COMMENT, &[REGEX]);
    let raw_quote = "!Regex \"a\\\" # outside\n";
    let trace = observe(&set, raw_quote, None);
    assert_role(raw_quote, &trace, "a\\", REGEX, &[ESCAPE]);
    assert_role(raw_quote, &trace, "# outside", COMMENT, &[REGEX]);
}

#[test]
fn raw_regex_triples_are_raw_across_all_delimiters_gaps_and_newline_forms() {
    let set = syntax_set();
    for delimiter in ["\"\"\"", "'''"] {
        for gap in ["", " ", "\t"] {
            for eol in ["\n", "\r\n", "\r"] {
                let source = format!(
                    "!String {{pattern: !Regex{gap}{delimiter}{eol}  (?x)\\d+\\\\x # raw field: | !Tag{eol}{eol}  a \\{delimiter} b{eol}  c \\\\\\{delimiter} d{eol}  last\\{eol}  {delimiter} # trailing{eol}}}{eol}next: 12 # outside"
                );
                let trace = observe(&set, &source, None);
                assert_role(&source, &trace, "!Regex", TYPE, &[REGEX]);
                for body in ["(?x)\\d+\\\\x # raw field: | !Tag", " b", " d", "last\\"] {
                    assert_role(
                        &source,
                        &trace,
                        body,
                        REGEX,
                        &[ESCAPE, COMMENT, TYPE, PROPERTY, OPERATOR],
                    );
                }
                assert_role(&source, &trace, "# trailing", COMMENT, &[REGEX]);
                assert_role(&source, &trace, "next", PROPERTY, &[REGEX]);
                assert_role(&source, &trace, "# outside", COMMENT, &[REGEX]);
            }
        }
    }
}

#[test]
fn regex_dispatch_is_exact_and_horizontal_only_while_other_constructors_are_ordinary() {
    let set = syntax_set();
    for prefix in ["!Regex\n", "!app.Regex ", "!RegexExtra ", "!Date "] {
        let source = format!("{prefix}\"\"\"\n  a \\\\ b # text\n  \"\"\" # outside\nnext: 1\n");
        let trace = observe(&set, &source, None);
        assert_role(&source, &trace, "  a ", STRING, &[REGEX]);
        assert_role(&source, &trace, "\\\\", ESCAPE, &[REGEX]);
        assert_role(&source, &trace, "# text", STRING, &[COMMENT, REGEX]);
        assert_role(&source, &trace, "# outside", COMMENT, &[STRING, REGEX]);
    }
    let source = "{date: !Date \"2026-10-29\" time: !Time \"09:30:00+01:00\" deadline: !DateTime \"2026-10-29T10:30\" cron: !Cron \"0 0 9 * * FRI#3\"}\n";
    let trace = observe(&set, source, None);
    for constructor in ["!Date", "!Time", "!DateTime", "!Cron"] {
        assert_role(source, &trace, constructor, TYPE, &[REGEX]);
    }
    for payload in [
        "2026-10-29",
        "09:30:00+01:00",
        "2026-10-29T10:30",
        "0 0 9 * * FRI#3",
    ] {
        assert_role(
            source,
            &trace,
            payload,
            STRING,
            &[REGEX, COMMENT, NUMBER, OPERATOR],
        );
    }
}

#[test]
fn cached_complete_line_streams_match_uninterrupted_state_and_scopes() {
    let set = syntax_set();
    let template = "{é: \"😀\"\ntext: \"\"\"\n  # ordinary\n  \"\"\" # one\npattern: !Regex '''\n  \\d+ # raw\n  ''' # two\nnext: 1\n}";
    for eol in ["\n", "\r\n", "\r"] {
        let source = template.replace('\n', eol);
        let baseline = observe(&set, &source, None);
        let line_count = physical_lines(&source).count();
        for pause in 0..line_count {
            assert_eq!(
                observe(&set, &source, Some(pause)),
                baseline,
                "pause {pause}, ending {eol:?}"
            );
        }
        assert_role(&source, &baseline, "# ordinary", STRING, &[COMMENT]);
        assert_role(&source, &baseline, "# raw", REGEX, &[COMMENT, ESCAPE]);
        assert_role(&source, &baseline, "next", PROPERTY, &[STRING, REGEX]);
    }
}

#[test]
fn consumer_palettes_style_standard_roles_without_grammar_owned_rgb_expectations() {
    let set = syntax_set();
    let syntax = set.find_syntax_by_token("daml").unwrap();
    let themes = ThemeSet::load_defaults();
    let source = "{name: !app.Type \"value\" count: 12 enabled: true} # comment\n";
    let trace = observe(&set, source, None);
    for theme_name in ["base16-ocean.dark", "base16-ocean.light"] {
        let theme = &themes.themes[theme_name];
        let palette = Highlighter::new(theme);
        let mut consumer = HighlightLines::new(syntax, theme);
        let styled = consumer.highlight_line(source, &set).unwrap();
        assert_eq!(
            styled.iter().map(|(_, text)| *text).collect::<String>(),
            source
        );
        for (needle, role) in [
            ("name", PROPERTY),
            ("!app.Type", TYPE),
            ("value", STRING),
            ("12", NUMBER),
            ("true", "constant.language"),
            (":", OPERATOR),
            ("{", "punctuation"),
            ("# comment", COMMENT),
        ] {
            assert_role(source, &trace, needle, role, &[]);
            let position = source.find(needle).unwrap();
            let scopes = &trace
                .ranges
                .iter()
                .find(|range| range.bytes.contains(&position))
                .unwrap()
                .scopes;
            let expected = palette.style_for_stack(scopes);
            let mut offset = 0;
            let actual = styled
                .iter()
                .find_map(|(style, text)| {
                    let range = offset..offset + text.len();
                    offset = range.end;
                    range.contains(&position).then_some(*style)
                })
                .unwrap();
            assert_eq!(actual, expected, "consumer palette controls {role}");
            if [STRING, NUMBER, COMMENT].contains(&role) {
                assert_ne!(
                    actual,
                    palette.get_default(),
                    "standard theme renders {role}"
                );
            }
        }
    }
}
