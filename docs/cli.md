# Check, resolve, and explain configuration

The source-built `daml` command validates configuration, completes defaults, and explains checked values. It uses the same native compiler, schema registry, and resolver as the library. The existing `daml-fmt` and `daml-lsp` commands still own formatting and editor syntax support.

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
```

Each operation parses, completes, and validates the whole configuration before reporting success. A root assertion such as `!app.Service`, or `--expected app.Service`, must select a root contract. An expected contract does not waive a different explicit assertion; both remain requirements. These commands do not offer a syntax-only success mode.

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

Parser, schema, exact-number, and cumulative resolution limits remain active. Module input is limited to 10,000 files and 8 MiB of aggregate source. Explanation additionally reserves eager validator-output and metadata work before allocation; it can exhaust the remaining budget sooner than resolution alone. It never returns a truncated explanation as success. Successful output is buffered under an 8 MiB cap before being written.

| Exit status | Meaning |
|---|---|
| 0 | Success; `check` has no result output |
| 1 | Configuration, schema, admission, pointer, representation, or output-limit failure |
| 2 | Invalid invocation or file/stream I/O failure |

Diagnostics go to stderr; only successful result data goes to stdout. Engine failures retain available instance paths, byte spans, default origins, schema locations, and causes. Admission or representation failures do not emit partial results. A failing output stream may already have accepted a prefix and returns 2.

The CLI never overwrites configuration files. If you redirect output, use another file: redirecting over the input path lets the shell truncate it before DAML opens it.
