# Format a DAML document

The formatter reflows data expressions or native definitions to a target width
without changing decoded values, tags, mapping order, or numeric spellings. It
uses a parsed document and its source; it does not resolve schemas, materialize
defaults, or compile native definitions.

## Data library API

```rust
use daml::{FormatOptions, format, parse};

let document = parse("{long_key:{value:true}}")?;
let output = format(&document, FormatOptions { line_width: 24 })?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The output is:

```daml
{
  long_key: {
    value: true
  }
}
```

`FormatOptions::default()` targets 80 display columns. Any positive width is
supported; zero returns `FormatError::InvalidLineWidth`.
`FormatOptions::validate()` checks that contract before a caller performs I/O;
`format` also validates its options. Indentation is always two spaces.

Width is a layout target, not a token-splitting rule. Long names, strings,
comments, and key/tag/container-opening headers can exceed it. A larger width
can collapse the example to `{ long_key: { value: true } }`. Wider layouts do
not add commas or relax DAML's required entry separators.

## Native definition API

```rust
use daml::{FormatOptions, format_definition, parse_definition};

let definition = parse_definition("!Int { min: 1 } = 10")?;
let output = format_definition(&definition, FormatOptions { line_width: 24 })?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `format_definition` for native definition expressions. It preserves comments,
scalar spelling, and block-string contents without resolving referenced
definitions or validating bundle semantics. See the [native authoring
contract](native-authoring.md) for the native grammar and bundle behavior.

Positional definitions retain their explicit `!Tuple [ ... ]` constructor,
including comments around the opening bracket and positions. Whole-tuple
defaults remain after the closing bracket. Ordinary arrays and tagged arrays
remain data expressions; the formatter does not infer a tuple definition from
bare brackets.

## Command

```sh
cargo run --locked --bin daml-fmt -- --width 80 examples/application.daml
cargo run --locked --bin daml-fmt -- --check examples/application.daml
```

`daml-fmt [--check] [--width N] [FILE|-]` reads one file, or standard input when
no file or `-` is given. Normal mode writes formatted text to standard output.
It never overwrites the input file. There is no file discovery, configuration
search, or in-place write mode.

`--check` writes no formatted text and reports these exit statuses:

| Status | Meaning |
|---|---|
| 0 | Input already has canonical formatting. |
| 1 | Formatting would change the input; a diagnostic identifies the input. |
| 2 | Invalid options, invalid source/UTF-8, read failure, or formatting failure. |

Errors in normal mode also return 2. `--help` prints usage and returns 0.
The command bounds reads with the parser's default input-byte limit before
parsing. The library reuses an already parsed `Document`; a caller's chosen
parse limits still govern input acceptance. Reflow may grow the output, so an
application reparsing it must allow the resulting source length rather than
assume that formatting preserves byte counts.

## Presentation rules

- Empty containers are `{}` and `[]`. A single-entry mapping can become
  `{ key: value }`; a single-item sequence can become `[value]`. They expand
  when the layout does not fit.
- Multiple entries remain separated by physical newlines at every width.
  Children indent two spaces; closing braces/brackets align with the container.
- Mapping keys retain their spelling and have no space before `:`. Container
  and block-string openings stay with their key. A scalar value can move to
  an indented next line when necessary.
- A tag and its payload remain on the same physical line. A bare empty tagged
  mapping remains bare; explicit braces are not removed or inserted for it.
- Comments retain their text and lexical order. Same-line comments stay after
  the preceding token; own-line comments stay on their own line. Outside
  strings, blank lines and other whitespace normalize to structural separators.
- Ordinary strings keep their source escapes. Literal and wrapped block bodies
  are not word-reflowed or re-encoded. Physical line endings normalize to LF.
  Entirely blank literal bodies strip horizontal whitespace according to the
  syntax contract; nonblank body indentation remains content after dedent. The
  closing delimiter aligns with its structural indentation without changing the
  decoded value. A block delimiter is never joined to a parent closing
  brace/bracket on the same line.
- A leading BOM is retained. Output ends in one LF.

Formatting is idempotent at the same width. Reparse equivalence includes exact
numeric lexemes, decoded strings, tags, mapping keys/order, and sequence order;
source spans necessarily change.

## Ownership and reused machinery

`src/format.rs` builds data and native definition documents for pinned MIT
`pretty` 0.12.5, reusing its Wadler-style layout solver and arena instead of
implementing a printer engine. Dynamic and signed nesting keep nonblank literal
content independent of structural indentation while aligning delimiters. The
data parser and formatter share one whitespace/comment scanner in
`src/parser.rs`; comments are read only from AST-delimited source gaps. No
second data grammar or parser is introduced.

The CLI accepts both data expressions and native definitions through the same
native syntax and formatter path. It remains a thin input/output adapter. See the
[reuse assessment](formatter-reuse.md) for alternatives and the
[edge-case record](edge-cases.md) for confirmed rules and historical
observations. The existing PLAN remains the implementation ledger; this guide
owns the formatter contract rather than introducing a parallel task system.
