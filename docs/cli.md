# Check, resolve, explain, and edit configuration

The source-built `daml` command validates configuration, completes defaults, explains checked values, and applies targeted source edits. It uses the same parser, native compiler, schema registry, and resolver as the library. The existing `daml-fmt` and `daml-lsp` commands still own formatting and editor support.

## Build and try it

From the repository root, build with the [pinned Rust toolchain](development/building.md):

```sh
cargo build --locked --bin daml
target/debug/daml --help
target/debug/daml check --bundle examples/tutorial examples/tutorial/data/provider.daml
target/debug/daml resolve --bundle examples/tutorial examples/tutorial/data/provider.daml
target/debug/daml explain --bundle examples/tutorial --path /adapter/timeout examples/tutorial/data/provider.daml
```

`check` succeeds silently. `resolve` prints the completed configuration, including timeout 30 and retries 3, without changing the input. `explain` identifies `/adapter/timeout` as a defaulted number, with the default's schema resource and pointer. Explain `/adapter/endpoint` to see a supplied value and its original UTF-8 byte span instead.

This command is available from source. It is not included merely by downloading an older native archive; [distribution](distribution.md) describes the released formatter and language server. Adding the command to release archives is separate release work.

## Commands and schema selection

```text
daml check [--bundle DIR | --module FILE ...] [--expected NAME] [FILE|-]
daml resolve [--bundle DIR | --module FILE ...] [--expected NAME] [--format daml|json] [FILE|-]
daml explain [--bundle DIR | --module FILE ...] [--expected NAME] [--path POINTER] [FILE|-]
daml edit --edits FILE [--bundle DIR | --module FILE ...] [--expected NAME] [CONFIG|-]
```

Each operation completes and validates the whole configuration before reporting success. `edit` first applies the requested source changes and validates the resulting document, so it can repair a syntactically valid but schema-invalid configuration. A root assertion such as `!app.Service`, or `--expected app.Service`, must select a final root contract. An expected contract does not waive a different explicit assertion; both remain requirements. These commands do not offer a syntax-only success mode.

- `--bundle DIR` opens that native bundle and its local imports.
- With file input and no explicit context, native discovery selects the nearest ancestor `daml.bundle` from the document's directory, not an unrelated cwd bundle.
- Repeat `--module FILE` for strict JSON [schema-module envelopes](schemas.md#supply-an-interchange-module). Supply all referenced resources. Do not combine modules with `--bundle`; modules do not trigger native discovery or resource fetching.
- Omitted FILE or `-` reads stdin and requires `--bundle` or `--module`. `--` ends option parsing for a filename beginning with `-`.

For example, check a configuration with an explicit expected contract and modules:

```sh
target/debug/daml check --expected demo.Application \
  --module examples/schemas/application.json \
  --module examples/schemas/network.json \
  --module examples/schemas/text.json examples/application.daml
```

Finite structural recursive contracts use the existing [bounded recursion rules](schemas.md). Invalid descendants, nonprogressing reference loops, and unending required-container completion remain errors.

## Choose the output representation

`resolve` defaults to canonical DAML. It preserves exact number values, intrinsic temporal/cron kinds, and assertion meaning through the contextual writer. It does not preserve comments, field order, whitespace, or every original scalar spelling. If a retained private assertion has no equivalent visible source name, writing fails rather than dropping or renaming that assertion.

Use `--format json` to explicitly select completed plain data:

```sh
target/debug/daml resolve --bundle examples/tutorial --format json examples/tutorial/data/provider.daml
```

JSON retains exact integer and decimal tokens, but omits assertions and projects Date, Time, DateTime, and Cron values to strings. It is not a lossless DAML round trip or an authored-data export: defaults are included. The CLI neither imports arbitrary JSON/YAML data nor supplies a bidirectional converter. See [conversion boundaries](conversion.md).

## Edit authored source

Use `edit` when you want to change supplied values without reformatting unrelated source or writing omitted defaults. Save this DAML batch as `edits.daml`:

```daml
[
  {
    op: "replace"
    path: "/adapter/endpoint"
    value: "https://new.example"
  }
  {
    op: "insert"
    path: "/retries"
    value: 5
  }
]
```

Write the result to a different file, never over the input path:

```sh
target/debug/daml edit --edits edits.daml --bundle examples/tutorial \
  examples/tutorial/data/provider.daml > provider.edited.daml
```

The endpoint changes and retries 5 becomes an explicit supplied value. The timeout default remains omitted. Other source bytes retain their comments, spelling, order and line endings.

The batch is DAML data, not JSON Patch. Each untagged operation map requires untagged string `op` and `path`. `replace` and `insert` require a normal DAML `value`; `remove` forbids one. Unknown fields or operations fail. `--edits FILE` is required exactly once for edit and forbidden for other commands. `--edits -` is rejected because stdin belongs only to the configuration input. `--format` and `--path` keep their existing resolve-only and explain-only meanings.

Paths use strict RFC 6901 syntax. They address explicit original-source values, not completed defaults. Replace requires an existing node; an empty pointer replaces the root. Insert appends an absent mapping field, inserts before an original array index, or appends at the original array length. Remove requires an existing mapping field or array item. Root insert/remove, implicit parent creation, `-` indices, duplicate/ancestor-descendant targets and multiple insertions at one original boundary are rejected. All operations use original array indices; earlier removals do not shift later targets.

Replacing a node replaces its complete assertion/value subtree, including comments inside it. Removing an entry preserves preceding and trailing comments outside that subtree, even when this leaves an orphan comment or blank line. The editor does not guess comment ownership from indentation. Existing bytes outside affected regions remain exact. Generated structural newlines use the parent container's first physical line-ending style, or LF when none exists.

The CLI compiles and validates the complete edited document, including assertions introduced by the edits, before returning authored source. It never serializes the resolved/default-completed tree. A bad operation, conflict, invalid final value or exhausted budget rejects the whole batch without successful stdout. Configuration, batch, schemas and bundle files remain unchanged.

The reusable Rust API exposes `Document::prepare_source_edits`, `Schemas::prepare_source_edits` and `SourcePatch::apply`. Prepared patches borrow their original source and reject a changed current snapshot before applying byte offsets. The CLI owns one immutable read and does not overwrite files; it does not claim to detect later disk changes.

## Read an explanation

`explain` always emits JSON. `--path` is an RFC 6901 instance pointer; omit it or pass an empty string for the root. `/adapter/timeout` selects a nested member, `/workers/0/name` selects an array element, and `~1` escapes a slash inside a literal key. Dots in a key are literal. Array indices cannot have signs or leading zeros. Malformed and absent paths fail explicitly.

The report contains:

| Field | Meaning |
|---|---|
| `path` | Selected instance pointer |
| `value` | Existing exact semantic tree: `kind` plus scalar `value`, sequence `items`, or mapping `entries`; optional `tag` retains name/binding |
| `origin` | `input` with UTF-8 `span` and optional source filename; `default` with `resource`/`pointer`; or `semantic` without invented source coordinates |
| `tagOrigin` | Separate assertion origin, when present |
| `constraints` | Applied direct and enclosing contracts, each with its actual `instancePath`, `schemaLocation`, and admitted `schema` |

Semantic numbers use strings in `value`, never JavaScript floating point. Temporal and cron values retain their intrinsic `kind`. Ancestor constraints are labeled at their ancestor instance paths; they are not presented as node-local facets. References remain references. A schema resource identifier is diagnostic identity, not permission to fetch it.

Explanation captures authoritative validator evaluation, including validation-only alternatives and nested assertions. Rejected alternatives are not advertised as applied contracts. Enclosing schemas can still describe their alternatives; their presence does not declare every branch selected. A successful explanation requires normal whole-document admission.

## Limits and failures

Parser, schema, exact-number, and cumulative resolution limits remain active. Module input is limited to 10,000 files and 8 MiB of aggregate source. Edit batches have an 8 MiB source cap and default core limits of 1,024 operations, 16 MiB admission work and 8 MiB output; supplied values also retain the semantic renderer's bounds. Explanation additionally reserves eager validator-output and metadata work before allocation; it can exhaust the remaining budget sooner than resolution alone. Neither operation returns a truncated result as success. Successful output is fully admitted under an 8 MiB cap before stdout writes.

| Exit status | Meaning |
|---|---|
| 0 | Success; `check` has no result output |
| 1 | Configuration, schema, admission, pointer, representation, or output-limit failure |
| 2 | Invalid invocation or file/stream I/O failure |

Diagnostics go to stderr; only successful result data goes to stdout. Engine failures retain available instance paths, byte spans, default origins, schema locations, and causes. Admission or representation failures do not emit partial results. A failing output stream may already have accepted a prefix and returns 2.

The CLI never overwrites configuration files. If you redirect output, use another file: redirecting over the input path lets the shell truncate it before DAML opens it.
