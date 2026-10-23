# A walkthrough of DAML's language choices

DAML makes structure and value conversion explicit. This walkthrough follows a document from its visible syntax through parsing, validation, formatting, and editor use. It separates confirmed language choices from implementation evidence. The syntax contract, schema guide, formatter guide, LSP guide, and edge-case record remain authoritative for exact behavior.

## Start with explicit structure

Braces and brackets own mapping and sequence membership. Indentation helps people read the tree, but does not create it.

```daml
{
  adapter: !network.Adapter
    timeout: 30
}
```

Here `adapter` is an empty mapping tagged `network.Adapter`; `timeout` is a sibling in the outer mapping. Indenting `timeout` does not move it into the adapter. Write braces to express that ownership:

```daml
{
  adapter: !network.Adapter { timeout: 30 }
}
```

A document has exactly one root value; empty or comment-only source is rejected. Empty mappings and sequences are written `{}` and `[]`. A mapping or sequence may contain one inline entry, but entries in a multi-entry collection need physical newlines outside strings and comments. Commas are not separators:

```daml
[1
2]
```

This is a two-item sequence. `[1, 2]` is rejected rather than interpreted as another syntax style. Requiring line breaks for multiple entries keeps separators visible without making indentation a second source of structure.

Keys are unquoted, nonempty tokens without whitespace, followed by `:`. Quoted keys are rejected. Unicode letters/numbers, subsequent combining marks, and `_ - . /` are allowed; digit-leading keys are strings. Dots/slashes in keys imply no lookup or file access. Tag segments use the same word characters but only `_ -` punctuation; dots separate segments. Names are exact, without normalization or case folding. The [syntax contract](syntax.md#structure-and-names) owns the complete alphabet.

## Tags mark values; they do not construct types

A single `!tag` can precede any value, including a nested mapping, sequence, scalar, or empty mapping. Tags are literal metadata. Parsing preserves unknown tags; a later validation pass decides whether a supplied schema set knows and accepts them. Parsing does not import code, instantiate an application type, or treat a tag as a cast.

A tag's explicit payload must begin on the same physical line as its name. A tag with no same-line payload denotes an empty tagged mapping. The next entry stays separate even if indented beneath it.

The earlier adapter example illustrates this boundary. To give the adapter fields, use an explicit mapping payload. Stacked tags are rejected. Parent constraints and a node's own tag assertion both apply during validation; a tag cannot waive the containing field's requirements.

## Choose a string form by the value you need

Ordinary double quotes use JSON string escapes. They suit short values and escape-sensitive data:

```daml
{
  message: "line one\nline two"
  symbol: "\uD83D\uDE00"
}
```

The decoded values are `line one`, a newline, then `line two`; and `😀`. Malformed escapes, lone UTF-16 surrogates, raw control characters, and physical newlines inside an ordinary string are errors. Ordinary single-quoted strings are not supported.

Triple double quotes preserve multiline layout after mandatory dedent. Triple single quotes wrap physical lines into one logical line by trimming each line, discarding blank lines, and joining the rest with one space. These are the only two multiline forms; they serve different purposes.

```daml
{
  literal: """
      release:
        name: daml
  """
  wrapped: '''
    Extract the release name.

    Return JSON only.
  '''
}
```

`literal` decodes to `release:\n  name: daml`. `wrapped` decodes to `Extract the release name. Return JSON only.` Use the literal form when internal line breaks or relative indentation matter. Use the wrapped form when source wrapping should not add line breaks to the value.

Both triple forms require the opener to be followed by optional spaces/tabs and a line ending. The matching closer occupies its own physical line, with optional indentation and then only spaces/tabs or a comment. The opening line ending and the one line ending immediately before the closing line are boundaries, not content. Additional blank lines inside the body remain data for triple-double strings; there is no chomping suffix. A matching unescaped triple delimiter inside the body is an error, not an inline terminator.

Triple-double dedent uses the minimum count of leading ASCII spaces among nonblank physical content lines. It does not use the closing delimiter's column or the enclosing collection's indentation. Tabs do not count toward the baseline and remain literal data. Dedent occurs before line continuation, so a physical line that continuation later consumes can still set the baseline.

An unescaped backslash immediately before an internal physical newline removes the backslash, newline, and following line's leading spaces/tabs. It inserts no space. Thus a space before the backslash remains in the result. Spaces or tabs between the backslash and newline are an error. `\n` inside triple text means two literal characters, not a newline escape; escaped backslashes and matching delimiters follow the triple-string rules.

For example, the second physical line below sets a two-space baseline before
continuation joins it to the first line:

```daml
"""
    first\
  second
    end
"""
```

The decoded value is `  firstsecond\n  end`: dedent removes two spaces from
each nonblank physical line, then joining removes the backslash, newline, and
remaining indentation before `second`. No space is inserted between `first`
and `second`.

Boundary removal does not discard extra blank lines inside the body:

```daml
"""

  x

"""
```

This decodes to `\nx\n`. The first and last line endings frame the string;
the additional blank lines remain content.

Entirely blank triple-double content strips spaces/tabs from each content line
while preserving internal line breaks. Tabs on any line containing non-whitespace
remain literal data and never contribute to the dedent baseline.

## Keep scalar types explicit

DAML recognizes `true`, `false`, and `null` as explicit values. Numbers use JSON decimal syntax, and their source lexemes remain exact: `1e2` stays `1e2`, and `-0` stays `-0`. Invalid forms such as `01`, `1.`, or `1e+` fail. Plain words do not turn into booleans, dates, or strings; write strings with a string delimiter.

Repeated exact mapping keys are errors. A second root value is an error. YAML document markers, anchors, aliases, block collections, `|` and `>` scalars, and YAML-style implicit typing are not alternate DAML features. Parsing has no interpolation, include, environment lookup, schema import, or application execution.

## Parse first, validate separately

Parsing produces a source-backed document with explicit values, literal tags,
numeric spellings, comments, and original byte spans. It does not need a schema.
Schema loading, default completion, and validation are separate operations.

Modules use the low-level JSON-compatible interchange format described in the
[schema guide](schemas.md). It accepts named type schemas and explicitly
registered resources. Schema documents can use JSON Schema's `$id`, `$ref`, and
other dollar-prefixed keywords, but final DAML data keys cannot contain `$`.
Use `SchemaModule::from_json` or `SchemaModule::from_value` when supplying JSON
interchange data; `from_value` accepts an already parsed value.

Native definitions and bundles are the authoring path described in
[native definitions and bundles](native-authoring.md). The native compiler
lowers them in memory to JSON Schema constraints and completion metadata; it
does not generate schema files. The interchange APIs remain available to
consumers that already have schemas.

Sequence constraints use explicit constructors: `!List { items: !Int }` applies
one constraint to every element; `!Tuple [!Int]` declares exactly one integer
position. Tuple has no length/uniqueness facets or scalar position defaults.
Its whole-value default follows the complete definition, as in
`!Tuple [!Int] = [1]`. Bare brackets continue to represent ordinary sequence data.

Native completion creates required missing records and maps from `{}` and lists
from `[]`, then applies applicable defaults and validates. It does not invent
primitive values, map entries, list elements, or implementation tags.
Optional absent values remain absent. An explicitly tagged supplied object
replaces its contextual object preset, even when its tag selects the same
definition; defaults belonging to the selected definition still apply, and
containing constraints remain active. Supplied arrays and scalars replace
defaults. These rules do not mutate the parsed input document.

## Reformat without changing values

`daml-fmt` reuses the parsed document and formats structure; it does not parse a second grammar, resolve schemas, or apply defaults. It preserves comments, key order, tags, numeric spellings, and decoded string values. Multiple entries remain separated by newlines at every width, and a tag stays on the same line as its payload. Block-string bodies are not word-reflowed or re-encoded. A wider layout can collapse a legal single-entry container, but cannot change ownership or insert commas.

## Use syntax diagnostics and formatting in an editor

The optional stdio `daml-lsp` reuses the parser and formatter. It reports syntax diagnostics and whole-document formatting only. It does not validate schemas, discover schema files, provide completion or hover, or read source files. It accepts full-document changes and tracks versions; clients must discard a formatting result if their buffer changes after the request.

The parser reports source spans in UTF-8 byte offsets. LSP ranges use the encoding negotiated with the client: UTF-8, UTF-16, or UTF-32, with UTF-16 fallback. A supplementary Unicode character therefore occupies four bytes, two UTF-16 units, or one UTF-32 unit. Protocol coordinates are not byte offsets or display columns.

## Resource limits and errors

Parsing stops at the first error and reports its category and original
half-open byte span. Default bounds are 8 MiB of input, 1 MiB per scalar
token/key/tag, 100,000 value nodes, and 128 nested mappings or sequences.
Exceeding a bound returns a structured error rather than a partial document;
custom limits can lower these bounds or raise size and node limits, but cannot
raise the nesting ceiling.

## Evidence and settled choices

The edge-case record contains 45 canonical historical probe inputs with recorded
status, output, errors, and byte spans. It is compact evidence, not exhaustive
coverage; the current syntax contract owns the final key/tag grammar.

Confirmed syntax choices include explicit braces and brackets, newline entry
separation without commas, the two triple-string forms, mandatory space-based
dedent, and unknown tags surviving parsing for later validation. Structural
interchange uses registered JSON Schema references and structural compatibility;
contracts, `implements`, `x-daml-tag`, and `x-daml-slot` are not part of that
API.

Earlier walkthrough and plan notes recorded native bundles, definition
overrides, and container completion as accepted direction still awaiting
implementation. Those statements describe the earlier state, not the current
contract: native definitions and bundles are now specified in
[native-authoring.md](native-authoring.md). Their rules are distinct from
low-level interchange completion. The syntax alphabet and triple-string edge
cases are now settled in the owning [syntax](syntax.md), [schema](schemas.md),
[edge-case](edge-cases.md), [formatter](formatting.md), and [LSP](lsp.md)
contracts.
