# Formatter reuse assessment

Evidence reviewed on 2026-10-18 against DAML `79241ba`. This is a source-based
recommendation, not an implemented formatter, dependency approval, grammar
change, performance comparison, or project-license decision. No upstream code
or fixtures were copied.

## Keep language decisions separate from presentation

A later language edge-case pass will reconsider triple-string framing, blank
lines, dedent, continuations, and the deferred key/tag alphabets. Formatter work
must preserve the implemented meanings until those contracts are explicitly
revised; formatting preferences must not become accidental parser rules.

The current [syntax contract](syntax.md#triple-string-boundaries) requires a line
ending after either triple opener, allowing only spaces/tabs before it. The
opening line ending and one line ending immediately before the closing line
are framing, not string data. For example:

```daml
"""
  hello
"""
```

The decoded value is `"hello"`, without a leading or trailing newline. A rebuilt
`inspect` consumer at the reviewed DAML revision also observed:

| Input variation | Current result |
|---|---|
| An additional blank line after the opening line | `"\nhello"` |
| An additional blank line before the closing line | `"hello\n"` |
| Inline `"""hello"""` | Rejected as invalid string syntax |
| CRLF delimiter/content line endings | `"hello"` |
| Empty block, opener immediately followed by its closing line | `""` |

These are current observations, not resolutions of the later design review.
That review should use explicit input/decoded-value/rejection examples rather
than settle each case indirectly while implementing a formatter.

## What can actually be reused

| Candidate | Primary-source facts | DAML recommendation |
|---|---|---|
| **`tiny_pretty` 0.4.5** | Rust Wadler-style document printer with nesting, groups, hard lines, and width-dependent breaks. MIT; its published manifest has an optional `unicode-width` dependency and no required normal dependency. `Doc::text` must not contain line breaks. [API][tiny-api], [document primitives][tiny-doc], [manifest][tiny-manifest] | First layout-engine candidate if DAML needs line fitting or structural reflow. Do not write our own general document algebra or width solver. It still needs DAML-specific rules and deliberate handling of verbatim multiline strings. |
| **`pretty` 0.12.5** | Rust Wadler-style printer with boxed, reference-counted, and arena allocation APIs. MIT; required dependencies include `arrayvec`, `typed-arena`, and `unicode-width`. [API][pretty-api], [manifest][pretty-manifest] | Viable alternative when its allocator/builder API better fits a demonstrated requirement. No executed performance evidence favors it or `tiny_pretty`. |
| **Pretty YAML 0.6.0**, revision `c4068f4` | Rust YAML formatter. Its formatter package uses `yaml_parser`, Rowan, and `tiny_pretty`; its dprint integration lives in a separate package. MIT. [formatter manifest][yaml-manifest], [plugin manifest][yaml-plugin], [README][yaml-readme] | Closest concrete Rust implementation reference, especially the separation of formatter library and host integration. Not a drop-in DAML formatter: its parser and rules target YAML. |
| **dprint / `dprint-core` 0.71.0** | Host supports Wasm and process plugins; plugins need not use its printer. Native Rust `dprint-core::formatting` provides print items and rendering. Its MIT manifest keeps formatting, Wasm, process communication, and async runtime features distinct. Host CLI already provides file selection, ignore handling, stdin, `check`, and incremental formatting. [plugins][dprint-plugin], [core API][dprint-api], [manifest][dprint-manifest], [CLI][dprint-cli] | Candidate for reusing surrounding tooling rather than writing our own formatting platform. A thin plugin should call the same Rust formatting library. Adopting dprint as host does not require adopting its printer, or vice versa; Wasm is not required for native core use. |
| **Topiary**, revision `7c61712` | Generic formatter driven by Tree-sitter queries, producing leaf-content/formatting atoms and indentation directives. Its CLI checks idempotence by default. [design][topiary-design], [pipeline][topiary-pipeline] | Strongest inspiration for token-preserving indentation. Direct adoption requires a DAML Tree-sitter grammar rather than consuming the existing native parser's tree. Avoid introducing a second syntax authority solely to obtain its engine. |
| **Prettier** | AST printers produce a document IR, which the engine measures and lays out. Plugins still supply language parsing/printing and optional comment attachment/printing behavior. [architecture][prettier-design], [plugin contract][prettier-plugin] | Borrow the separation of language rules from layout and its comment-handling lessons. A Prettier plugin remains an option, but does not eliminate DAML-specific work and adds a JavaScript integration boundary to the Rust core. |

## Smallest approach for the current scope

**Recommendation:** begin with parser-guided, source-preserving indentation
edits, borrowing Topiary's leaf-preservation principle. Do not introduce a
second parser or a generic layout engine just to change leading whitespace.
Keep string token contents, comments, names, tags, and numeric spellings intact.
Any required trivia classification belongs with the existing parser/source
model, not a parallel regex-based grammar. The current model retains the full
source and node spans; comments are not yet structured trivia nodes.
[Source contract](syntax.md#source-model-and-errors), [parser trivia handling](../src/parser.rs)

If the accepted formatter scope includes width-dependent layout, select an
existing Rust printer instead of implementing that machinery. `tiny_pretty` is
the first candidate to try, with `pretty` and `dprint-core` as concrete
alternatives. This is an engineering preference based on the APIs and dependency
boundaries above, not a measured claim about speed or total implementation size.

Two DAML constraints must survive any engine adapter:

- Collection entries require physical newlines. A generic soft break that can
  flatten to a space cannot separate multiple entries.
- A tag's payload starts on the same physical line. Moving that payload to the
  next line changes the parse; strings also have their own newline/dedent rules.

These follow from [the syntax contract](syntax.md#structure-and-names), not the
printer's preferred style. YAML/JSON formatting followed by comma removal or
quote substitution is not a valid adaptation strategy.

For repository-wide CLI/editor integration, consider a dprint plugin before
building file discovery, ignore handling, incremental formatting, or a broad
configuration system ourselves. Keep that adapter outside the portable Rust
formatter. A small standalone file/stdin command is still possible; this note
has not compared its implementation cost with a dprint adapter.

Before adding a printer dependency, prove the chosen approach on one nested
commented document containing bare tags, a literal block with significant blank
lines/relative indentation, and wrapped text. Require idempotence and unchanged
parsed values/tags. Do not retain two formatter implementations after that
choice. No candidate printer or plugin was compiled or benchmarked for this
assessment; only the current DAML string-boundary cases were executed.

[tiny-api]: https://docs.rs/tiny_pretty/0.4.5/tiny_pretty/
[tiny-doc]: https://docs.rs/tiny_pretty/0.4.5/tiny_pretty/enum.Doc.html
[tiny-manifest]: https://docs.rs/crate/tiny_pretty/0.4.5/source/Cargo.toml
[pretty-api]: https://docs.rs/pretty/0.12.5/pretty/
[pretty-manifest]: https://docs.rs/crate/pretty/0.12.5/source/Cargo.toml
[yaml-manifest]: https://github.com/g-plane/pretty_yaml/blob/c4068f4dbf96f59c631c2259b42890e072f8b7c1/pretty_yaml/Cargo.toml
[yaml-plugin]: https://github.com/g-plane/pretty_yaml/blob/c4068f4dbf96f59c631c2259b42890e072f8b7c1/dprint_plugin/Cargo.toml
[yaml-readme]: https://github.com/g-plane/pretty_yaml/blob/c4068f4dbf96f59c631c2259b42890e072f8b7c1/README.md
[dprint-plugin]: https://dprint.dev/plugin-dev/
[dprint-api]: https://docs.rs/dprint-core/0.71.0/dprint_core/formatting/
[dprint-manifest]: https://docs.rs/crate/dprint-core/0.71.0/source/Cargo.toml
[dprint-cli]: https://dprint.dev/cli/
[topiary-design]: https://topiary.tweag.io/book/
[topiary-pipeline]: https://github.com/topiary/topiary/blob/7c6171250504e00e78f26fd4408ae0e193a61954/docs/book/src/reference/formatting-pipeline.md
[prettier-design]: https://prettier.io/docs/technical-details
[prettier-plugin]: https://prettier.io/docs/plugins
