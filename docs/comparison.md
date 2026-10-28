# DAML and neighboring formats

DAML, YAML, JSON, and JSON plus JSON Schema solve overlapping but different problems. DAML pairs a constrained configuration syntax with native structural definitions and completion. YAML prioritizes a compact human-authored representation and broad ecosystem support. JSON prioritizes a small, widely implemented interchange format. JSON Schema adds a separate validation language to JSON; it does not change JSON syntax.

Start with the [tutorial](tutorial.md), then use the [syntax contract](syntax.md), [native authoring contract](native-authoring.md), and [schema reference](schemas.md) for exact rules.

| Concern | DAML | YAML | JSON | JSON + JSON Schema |
|---|---|---|---|---|
| Syntax | Braced mappings and bracketed sequences; explicit string and number forms; physical newlines separate entries. | Indentation-sensitive block forms and flow collections, with many scalar and quoting forms. | Braced objects and bracketed arrays; quoted strings; commas separate entries. | Same JSON syntax. Schema documents use JSON syntax too. |
| Validation and completion | Native definitions express structural constraints and presets; DAML resolution completes and validates values. | YAML syntax alone defines no application schema or completion. Applications commonly pair it with separate validators. | JSON syntax alone defines no application schema or completion. | JSON Schema describes constraints. Defaults in standard JSON Schema are annotations; DAML's resolution behavior is not implied by the standard. |
| Extension | Native definitions compose structural constraints. The low-level schema API accepts a bounded JSON Schema profile. | Tags, aliases, directives, and schema behavior vary by YAML version and parser configuration. | No user-defined syntax extension. | Schema keywords and references extend the validation description, not JSON data syntax. |
| Ecosystem | DAML-specific parsing, authoring, and resolution tools. | Broad editor, parser, and application support, with version and parser differences. | Broad support across languages, APIs, and storage systems. | Broad tooling, but support for drafts and vocabularies varies. |
| Representability | JSON-compatible data shapes with DAML's explicit tags and syntax rules; not every YAML feature. | Supports richer syntax and graph features, including aliases and tagged nodes, subject to version and parser. | Objects, arrays, strings, numbers, booleans, and null. Object member names are arbitrary strings. | The schema can constrain JSON values, but JSON Schema does not turn arbitrary values into nominal types or define DAML's tag/completion semantics. |

## YAML depends on version and parser

“YAML” is not one uniform scalar interpretation across all deployed tooling. YAML 1.1 and 1.2 differ in areas such as implicit scalar resolution, and parsers may select different schemas or compatibility modes. Features such as aliases and custom tags also depend on the parser and application. A YAML document therefore needs an explicit version and parser context when exact cross-tool behavior matters.
| Representability | JSON-compatible data shapes with DAML's explicit tags and syntax rules; not every YAML feature. | Supports richer syntax and graph features, including aliases and tagged nodes, subject to version and parser. | Objects, arrays, strings, numbers, booleans, and null. Object member names are arbitrary strings. | The schema can constrain JSON values, but JSON Schema does not turn arbitrary values into nominal types or define DAML's tag/completion semantics. |
DAML intentionally defines a narrower syntax: it does not use indentation to build collections, infer dates, or accept anchors and aliases. That makes the grammar more limited than YAML, but removes those sources of parser configuration variation. See [DAML scalar rules](syntax.md#scalars-and-input).

## JSON keys are not DAML keys

JSON object member names are strings, so JSON can represent arbitrary keys such as `"a key with spaces"` or `"$schema"`. DAML data keys follow its own unquoted key grammar and cannot use every JSON string as a key. DAML tags also have explicit syntax and meaning that JSON does not provide.

JSON is a strong choice when consumers need the most direct interchange with existing libraries. DAML is a better fit when the authoring syntax and its structural definitions are part of the configuration contract. See [DAML names and structure](syntax.md#structure-and-names).

## JSON Schema is a separate contract layer

JSON Schema lets a consumer validate JSON against a separately authored schema. The ecosystem is broad, but supported drafts, vocabulary, reference behavior, and default handling depend on the implementation and application. The standard `default` keyword is an annotation; it does not require validators to insert values.

DAML's native definitions offer an authoring model tied to DAML's completion and validation rules. DAML also accepts a [bounded JSON Schema interchange profile](schemas.md), so existing schemas can be useful without implying full support for every JSON Schema feature.

## Choose by the boundary you need

Choose YAML when human editing, established ecosystem support, or YAML-specific features matter and parser behavior is controlled. Choose JSON when the primary need is standard data interchange and a separate validation layer is acceptable. Choose JSON plus JSON Schema when schema portability and existing JSON Schema tooling matter, after checking the required draft and vocabulary against the actual implementation. Choose DAML when explicit configuration structure, native structural contracts, and defined default completion justify adopting DAML-specific syntax and tools.
