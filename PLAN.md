# DAML implementation plan

Status: data parsing, native definitions and local bundle loading, structural
validation and completion, hardening, reflow formatting, the minimal stdio LSP,
binary distribution proof, and the isolated Python extension are implemented.
The standalone browser SDK also passes focused local acceptance for Olaf's
browser-authoring prerequisite. Private publication is a separate delivery step.
[Syntax](docs/syntax.md), [native authoring](docs/native-authoring.md),
[interchange](docs/schemas.md), the [Python API](docs/python.md), and the
[browser SDK](bindings/browser/README.md) own the current contracts.

## Target result

A standalone safe-Rust library that parses DAML into a source-backed document,
preserves deeply nested tags, and validates their assertions against independently
composed schema modules. A real consumer can inspect values, tags, and source
locations without a global discriminator union or vendor-specific core behavior.

Ship complete consumer-visible behavior in each implementation slice. A lexer,
AST skeleton, forwarding API, or passing empty test suite is not a milestone.

## User-confirmed requirements

- Explicit mapping/list structure inspired by KYAML, without commas.
- Keys are unquoted and contain no spaces. Quoted keys are not an escape hatch.
- `!tags` on nested nodes; an empty tagged mapping can omit `{}`. Fields of a
  nonempty tagged mapping belong inside its braces, not under a bare tag.
- Ordinary `"..."` strings.
- `'''...'''` for one logical line wrapped over multiple source lines: trim each
  source line, discard blank lines, and join remaining lines with one space.
- `"""..."""` for literal multiline text, with **mandatory dedent**. Preserve
  relative indentation and internal line breaks. No optional dedent flag.
- Backslash immediately before a physical newline can explicitly join literal
  string lines; joining inserts no space.
- No YAML `|`, `|-`, `|+`, `>`, `>-`, or `>+` string variants.
- Rust implementation, reusable by independent consumers. Keep later Python
  integration possible without embedding Python into the core.
- Reuse or port existing code when its technical fit and license justify it.
- Provide a DAML formatter, primarily for indentation, and an LSP after the
  parser/library core. These are planned deliverables, not optional ideas.
- Schemas are expressible in DAML as well as JSON. Explicit defaults complete
  partial objects recursively, followed by full validation of required fields.

DAML is its own grammar, not a YAML/KYAML subset. The
[reuse assessment](docs/parser-reuse.md) explains the mismatches, pinned sources,
and license lineage.

## Key and tag policy boundary

The final key/tag grammar is implemented by one shared Unicode 16.0 classifier.
Ordinary keys allow letters/numbers, subsequent combining marks, and `_ - . /`;
tag segments allow the word characters and `_ -`, with dots separating segments.
Digit-leading names are allowed. No case folding or normalization occurs.
Keys remain literal: dots/slashes imply no nesting, lookup, or filesystem access.
Only bundle-qualified tags acquire definition-path meaning during loading.

## First acceptance specimen

The complete parser must accept this document, not merely fragments of it:

```daml
!demo.Application {
  adapter: !network.Adapter

  tasks: [
    !demo.Task {
      request: !text.Request {
        instruction: '''
          Extract the release name.
          Return JSON only.
        '''

        context: """
            release:
              name: daml
        """
      }
    }
  ]
}
```

Expected observations:

- `adapter` is an empty mapping tagged `network.Adapter`, equivalent to
  `!network.Adapter {}`.
- `instruction` equals `"Extract the release name. Return JSON only."`.
- `context` equals `"release:\n  name: daml"`: common content indentation is
  removed, extra indentation is preserved, and boundary newlines are excluded.
- Tags remain attached to their own nodes, including nested records and lists
  when explicitly tagged. There is no injected `__type__` field.
- The source span for `context` identifies the original delimiter-inclusive
  bytes; it is not an offset into the decoded/dedented value.
- Parsing works without loading schemas. Semantic validation checks the tags
  later and never imports or invokes anything named by the document.

### Tagged mapping ownership

Braces establish nesting; indentation presents it. In this document, the first
`timeout` belongs to the adapter and the second belongs to the outer mapping:

```daml
{
  adapter: !network.Adapter {
    timeout: 30
  }
  timeout: 60
}
```

Without the adapter's braces it is an empty mapping, and the following key is
a sibling regardless of how it is indented. The formatter should make that
ownership obvious; parsing must not infer additional ownership from indentation.

### Mandatory dedent contract

Define and test the transformation in this order:

1. Recognize the complete triple-double delimiter and preserve raw source spans.
2. Exclude the opening and closing delimiter-boundary newlines.
3. Determine common leading indentation from nonblank **physical content lines**.
4. Remove that common indentation, preserving additional relative indentation.
5. Apply explicit backslash-newline joining, removing continuation-line leading
   whitespace without inserting a space. A space before the backslash stays data.

Joining before dedenting is wrong: it can remove the least-indented physical
line and change the baseline. The closing delimiter's column and enclosing
mapping/list indentation do not determine the baseline.

Required cases include root versus deeply nested equivalent strings; more-indented
lines; blank and all-blank strings; deliberate leading/trailing blank lines;
Unicode; LF/CRLF; continuations with and without a preceding space; a continuation
that consumes the least-indented line; and escaped delimiter/backslash collisions.
Every equivalent-nesting pair must decode to identical bytes while retaining its
different original source spans.

## Milestone 1 — freeze the small language contract and reuse boundary

**Result:** an implementer can write the parser without inventing language rules.

The initial native implementation now has a [syntax contract](docs/syntax.md)
and executable acceptance cases. It consulted the reuse research but copied no
upstream code/tests and introduced no dependency.

- Write a compact grammar and a fixture matrix with exact decoded values or
  structured errors. Carry the acceptance specimen and dedent cases into it.
- Mark each YAML-derived fixture as unchanged, adapted, or intentionally rejected.
  Record the original upstream case ID and selected immutable corpus revision.
- Settle semantically consequential structure/value rules in the syntax contract;
  keep current key/tag grammar in the syntax contract rather than a second table.
- Use the same hard fixtures to assess a candidate reuse seam. Prefer a small
  native scanner/direct parser unless a targeted adaptation or helper such as
  Winnow demonstrably simplifies the implementation. Do not build two complete
  parsers or reject a useful dependency on principle.
- Before copying or porting code/tests, record upstream revision, copied paths,
  destinations, local changes, and required license/copyright notices. Do not
  import a full parser merely to avoid attributing a few useful routines.

The implemented decisions and limits are owned by the syntax contract, not a
second grammar table here. It fixes explicit collection boundaries, precise
decimal lexemes, JSON-style ordinary escapes, both triple-string forms, mandatory
dedent before continuation, UTF-8/line endings, structured errors, and resource
bounds. The native-authoring slice subsequently fixed the name alphabets.

**Gate:** confirmed structure/value spellings have exact acceptance outcomes,
imported material (if any) has provenance, and there is one production parsing
path. Neither final name alphabet is a blocker. YAML is a source, not DAML's
acceptance oracle.

## Milestone 2 — complete syntax-to-document parsing

**Result:** a Rust consumer parses the complete specimen and gets correct values,
nested tags, and original-source locations.

- Implement the selected grammar end to end: collections, scalars, both triple
  strings, comments, tags, empty tagged mappings, and complete-input consumption.
- Keep one source owner per document; retain ranges instead of source copies per
  token/node or references into sibling owned fields. Allocate decoded strings
  when escapes, wrapping, or dedent require changed content.
- Preserve tags, node spans, and comment/source information sufficient for later
  diagnostics or formatting. A lossless green-tree framework is not required.
- Return structured errors with original byte spans for malformed delimiters,
  separators, escapes, tags, keys, and numbers. Validate UTF-8 slice boundaries;
  source excerpts must not panic. Report EOF and multiline-string errors at
  useful locations, not only at the end of the file.
- Enforce the selected resource bounds from the first public parser entry point.
- Add a small runnable consumer example, such as
  `cargo run --example inspect -- examples/application.daml`, plus integration
  tests and executable public API examples. This is verification, not a separate
  production CLI product.

**Gate:** complete positive and negative fixture matrix passes; nested dedent
invariance and Unicode byte-span tests pass; the consumer example runs; no
schema registry is required just to parse. `./scripts/check` passes.

## Milestone 3 — structural validation and schema modules

**Implemented:** `SchemaModule` accepts an explicit interchange envelope with
`types` entries containing `{ schema: ... }` and registered `resources`
containing `{ uri: ..., schema: ... }`. Ordinary JSON Schema constraints and
registered references define compatibility structurally. There are no
contract-membership declarations or DAML-specific tag/slot validator keywords.
See [the schema guide](docs/schemas.md) for runnable examples and exact limits.

- A containing schema's constraints and a data tag's selected schema both apply.
- Untagged values may satisfy structural requirements. Unknown explicit tags
  remain errors, and tagged descendants are checked under open containers.
- Preserve exact numbers, explicit default completion, source/default origins,
  offline resource bounds, and existing schema-position classification.
- Reject removed `x-daml-tag` and `x-daml-slot` keywords at actual schema
  positions. Literal values and property names with those spellings remain data.
- Keep `x-daml-default-tags` for attaching explicit assertions to generated
  defaults; it does not declare membership.
- Consumers own conversion from foreign schema formats. The interchange module
  is not final native bundle authoring syntax.

**Gate:** registered schemas validate compatible differently named values
without membership declarations; incompatible structure and invalid data tags
fail at their value paths. Standard schema references and applicable defaults
remain covered by consumer examples and checks.

## Native authoring and bundle loading

**Implemented:** the [native authoring contract](docs/native-authoring.md) owns
definition syntax, one-type files, `daml.bundle`, local alias maps, private
imports, canonical-root identity, nested bundle boundaries, default replacement,
and completion. The executable corpus is `examples/native/cases.json`.

The accepted decisions were consolidated out of this plan rather than kept as
a second grammar. Their important tradeoffs remain:

- Bundle-local imports avoid repeated per-file declarations and dependency
  leakage. Nested manifests require explicit imports rather than route-dependent
  scope switching. Aliases are spellings, never nominal compatibility checks.
- `:` declares field constraints; `=` supplies a whole definition's default.
  Scalar field defaults are grouped in the containing record's preset, not
  scattered across inferred or typed member declarations. Nested container
  and metadata item/value presets remain attached to their own definitions.
  Default data uses `:` rather than declaration syntax.
- Records are open unless prefixed `closed`. Postfix closure, `as const`, and
  closed-by-default plus `...` were rejected. Referenced constraints conjoin;
  unrestricted derivation/weakening would violate structural contracts.
- Definition overrides replace a default component atomically; configuration
  completion prefills untagged objects. Explicitly tagged supplied objects
  discard the contextual preset but retain their selected definition's defaults.
- Required containers complete leanly; optional omissions stay absent.
  No primitive values, implementation tags, map entries, or list positions are
  guessed. Lists replace rather than index-merge.
- Inclusive unions accept only unambiguous semantic completion. Alternative
  order never selects a fallback. Known contradictory defaults fail compilation;
  partial defaults are allowed without a general satisfiability solver.
- In-memory lowering reuses the existing validator. Generated JSON Schema files,
  a second validator, published type IDs, and a package/network resolver are
  unnecessary for the accepted local bundle contract.

### Structural refinement

Normal path examined: source admission → shared parser/name rules → bundle-local
reference loading → native lowering → existing schema admission/composition →
default completion/validation → source-preserving formatting or LSP transport.

Consolidation: default-check projection now uses the schema profile's canonical
child-position classifier; removed its duplicate keyword table. Native and data
formatters share layout/trivia/string mechanics. Bundle filesystem admission
remains separate from schema validation and application construction.

Simplification: native lowering carries a borrowed source context instead of
repeating source/root/file argument triples. Completion layers are named rather
than an opaque tuple. The shared name classifier uses an ASCII fast path and
one Unicode category lookup per non-ASCII character. Removed compiler-side
revalidation of keys already admitted by the private syntax AST.

Post-review refinement: member-default replacement is normalized into cached,
per-use schema projections, not resolver ordering flags. One fallible recursive
owner caches changed and unchanged projections and bounds clone nodes and scalar
bytes before copying. Lowering depth is cumulative across expression/reference
traversal. Conditional alternatives retain both inherited member defaults and
ancestor-preset components, including through root-default-free projections.
Effective-default admission delegates constraints to the engine and shares property
and item selection with runtime completion, including overlapping map/named and
homogeneous/positional constraints. All declared default assertions are validated
centrally, including nested tags under grouped Any. Custom reference, applicator,
child, and preset walks share cumulative work/depth admission before the engine.
Both mutable traversal APIs use one classified child loop. Normalization carries
one borrowed inherited-state view and reuses the budget's borrowed source path.
Entry tags stay borrowed until unique binding registration. Completed schema
maps/arrays move into module envelopes instead of being serialized into copies.
Compiler materialization uses the same cumulative node/scalar-byte admission for
projections, lowering, presets, retained checks, and resolved default nodes. It
charges before cloning. An actual 600-reference, one-megabyte-default consumer
under a 192 MiB process cap now returns a structured compiler-limit error at
`A31.daml`; before this admission the process aborted on allocation.

Native optional presence intent is admitted once and consulted only for missing
values; supplying an object still uses its presets. Low-level interchange
optional defaults keep their existing behavior. Grouping preserves default
replacement/unset and optional-default rejection. Reflow keeps triple delimiters
on their own lines, including inside groups.
Malformed repeated assignments reject during parsing instead of reaching printer
invariants. `closed` renders intervening comments through the shared trivia owner.
Named defaults project an inherited map-value fallback at that key while retaining
the fallback for unrelated map entries.

Independent review's counterexamples now have regressions: adjacent tagged data
payloads in tooling, inherited member replacement/removal, recursive narrowed
defaults, contradictory union-branch defaults, optional references with presets,
and declaring-file diagnostics. Parent verification also exercises partial-object
replacement and branch-scoped overrides through a further reference.

Acceptance covers private imports and generated-tag bindings, explicit-tag preset
replacement, lean containers, list/item defaults, tuples, forbidden members,
invalid inherited defaults, nested boundaries, source limits, lossless filesystem
identity, and meaning-preserving native reflow.

Verification exercised on Linux:

- `scripts/check`: portable build, all-feature Clippy/tests/doctests, warning-clean
  rustdoc, and root/fuzz formatting.
- Default, all-feature, and fuzz dependency-policy graphs; fuzz Clippy.
- `scripts/fuzz-smoke 1000`: parse, interchange, resolution, and formatting under
  AddressSanitizer, including native syntax/reflow and data-superset properties.
- Runnable native bundle consumer with omitted, patched, and explicitly tagged
  data; inherited member replacement and contradictory-default rejection.
- Actual native formatter/check and stdio LSP open/diagnostics/format/shutdown.
- Migrated JSON interchange consumer and external adapter example; local
  maintained-document links.
- Bounded native-consumer probes: unchanged diamond graph reaches the engine's
  expanded-work rejection; many distinct overrides hit projection clone
  admission; warmed references hit graph-depth admission; list/map/tuple inherited
  defaults fail under contradictory conjunctions. Temporary inputs are removed.

The full gate uses `CARGO_BUILD_JOBS=1` on this host after one parallel linker
launch failed with OS error 11. No test or contract was weakened. These checks
do not establish cross-platform runtime behavior, native filesystem fuzzing,
complete native JSON Schema coverage, or semantic workspace LSP integration.

## Milestone 4 — harden and ship the standalone library

**Implemented hardening baseline:** retained parse/schema/resolve fuzz targets,
bounded AddressSanitizer campaigns, licensed YAML/DAML corpus cases,
cumulative-budget regressions, and release time/process-RSS measurements.
See [the measured evidence](docs/hardening-baseline.md) and
[reproduction commands](docs/hardening.md). The private native release uses the
MIT license and complete native legal payload; registry `publish = false` remains.

**Result:** an adversarial document cannot turn routine parsing/validation into
an uncontrolled panic, hang, unbounded recursion, or unintended execution.

- Expand the selected YAML corpus and DAML-specific regressions; preserve
  intentionally incompatible cases as rejection tests, not unexplained skips.
- Fuzz the real parse/validate entry points once they exist. Start with bounded
  inputs and the actual fixture corpus; retain minimized reproductions of bugs.
  Exercise limits, unterminated strings, escape runs, Unicode boundaries, deep
  collections, duplicate keys, and recursive schema-reference policy.
- Test limit failures and invalid span handling deterministically. Prevent
  recursive schema definitions or unbounded validation work from bypassing parser
  bounds; choose supported reference semantics explicitly.
- Measure representative small configurations, long dedented/wrapped strings,
  and deeply tagged documents at increasing sizes. Record revision, corpus,
  environment, time, and memory observations. Investigate accidental repeated
  scans/copies; do not promise throughput or zero-copy transformed strings
  without measurements, or add timing-sensitive CI thresholds.
- Document the actual grammar, public APIs, ownership, errors, limits, and
  deliberate YAML differences. Recheck notices and distribution terms for all
  code, fixtures, and dependencies. DAML's code is MIT licensed; keep
  `publish = false` for the selected private GitHub release. Registry publication
  requires a separate decision.
- Run the consumer smoke, targeted regressions, and full gate; review the final
  complete head rather than approving lexer/AST checkpoints.

**Gate:** all accepted library behavior is implemented, assertions work in an independent
consumer, no known correctness/security defects remain, bounded fuzz findings
are resolved, and documented commands work. Coverage is assessed by grammar and
failure behavior, not a decorative percentage or test count.

## Milestone 5 — source-preserving reflow formatter

**Result:** a contributor formats a real DAML document without changing its
meaning, comments, or string content.

**Implemented:** the [formatter API and command](docs/formatting.md) reuse
`pretty` 0.12.5 for width-sensitive layout. Parsing and formatting share the
existing source/AST and one trivia scanner; no second grammar is introduced.

- Use the same parsed syntax/source model, not another parser or YAML emitter.
- Apply one initial canonical indentation/layout policy; retain unquoted keys
  and explicit mapping ownership. Do not introduce a style-option framework.
- Preserve decoded literal/wrapped strings, relative content indentation, tags,
  and comments. Indenting a surrounding collection must not alter a dedented
  string's value. Do not rewrite ordinary string data as structural whitespace.
- Reflow legal single-entry containers to a target width. Multiple entries keep
  their required physical newlines, and tags keep same-line payload binding.

**Gate:** formatting is idempotent; parsing before and after formatting produces
the same values and tags; nested dedent/continuation and comment fixtures retain
their meaning. Exercise an actual file-formatting command, not only a formatter
helper in isolation.

## Milestone 6 — DAML language server

**Result:** an editor receives current-document syntax diagnostics and formatting
from the same library used by ordinary consumers.

**Implemented:** the optional [stdio LSP](docs/lsp.md) reuses synchronous
`lsp-server` transport and `lsp-types`, with versioned full-document state,
syntax diagnostics, negotiated Unicode ranges, and native reflow formatting.

- Reuse the checked parser and formatter for document open/change/close,
  syntax diagnostics, and document formatting. Schema diagnostics and module
  associations remain later work, subject to final schema-root sharpening.
- Convert original byte spans to the negotiated LSP position encoding correctly,
  including multibyte characters in values/comments.
- Associate results with document versions so stale work does not replace newer
  diagnostics or format a different revision. Keep workspace schema discovery
  separate from executing application code.
- Bundle discovery and relative-path rules are now settled in native authoring.
  Wiring those semantics into LSP diagnostics remains separately scoped; the
  current adapter still handles client-supplied syntax/formatting only.

**Gate:** a real LSP client session opens a document, introduces and fixes an
error, and requests formatting. Diagnostics clear on the correct revision,
Unicode ranges are correct, and edits preserve the parser's values/tags. No
editor-specific duplicate grammar becomes the language authority.

## Boundaries and implementation discipline

Keep one crate and private implementation modules until real boundaries justify
another package. Use safe synchronous Rust, small concrete functions/types,
explicit ownership, and structured errors. Add a dependency only for a concrete
job and apply the researched dependency-policy baseline when doing so. A crate's
unsafe prohibition is not evidence that transitive code contains no unsafe.

Do not silently substitute YAML preprocessing for parsing DAML, implement every
YAML feature, or lose tags by converting immediately to JSON/dictionaries.
Do not couple the grammar or schema owner to a consumer's wire format, callable
runtime, parameter templates, foreign references, or execution lifecycle.

The formatter and optional LSP follow the standalone library, not hidden
prerequisites for it. They share the checked parser/source information rather
than introducing another grammar or changing string rules.

Serde integration and a full command-line product remain separately scoped.
The separate Python package follows its documented API rather than earlier
illustrative sketches.

The standalone library has a retained hardening baseline, native bundle
authoring/loading, reflow formatter, and minimal stdio language server.
Schema resolution remains independent of application construction/execution.

The [edge-case record](docs/edge-cases.md) retains 45 historical runtime probes.
Triple-double boundary framing and mandatory dedent remain confirmed; the
current syntax contract supersedes provisional name-policy observations.

The [formatter reuse assessment](docs/formatter-reuse.md) records the selection
of `pretty` after proving source-sensitive literal and comment fixtures. Host
integration remains separate; no formatter platform or style framework is added.

## Next sequence

The [JSON Schema Draft 2020-12 audit](docs/json-schema-2020-12-audit.md)
separates backend validation, native expressibility, completion, and deliberate
profile limits. It found no sampled delegated-assertion defect and does not
justify another validator or blanket native parity.

Olaf's three apparent recursive components are closed-world discriminator
collector artifacts. Native tags and open structural protocols remove the
collector-to-implementation edges, along with `discriminator`, `__fn__`, and
`__type__`; they do not justify recursive definition support.

The native local-facet slice adds typed raw `!Regex`, String length/pattern,
List cardinality/uniqueness, and Map/Object property-count constraints. These
are basic local assertions independent of the current Olaf corpus. Native Regex
uses Rust syntax directly and full-string matching; standard JSON Schema
interchange retains its own pattern contract.

Consolidation closed after three independent review/repair rounds. Formatting
framing belongs to `Printer`; tag-aware/JSON-only equality belongs to one
`json_backend` comparator; the LSP admits syntax without copying whole buffers.
Normal unique completion and partial-default feasibility share the candidate
kernel. Branch-owned implicit-work discovery reuses the budgeted conjunction
owner and canonical property/item selectors; common work cannot activate inert
unions. Positional narrowing preserves item-schema defaults without promoting
replaced whole-array data into element fallbacks.

The [representative native conversion](examples/olaf/README.md) retains a recipe,
execution/optimizer definitions, a query, and independently scoped providers.
Three executable cases cover completed values/tags, five invalid shapes,
structural acceptance, and private-alias rejection. This is not an Olaf
integration: no consumer source/configuration changes, constructors, callable
dispatch, or runtime dependencies. Callable `parameters`/`returns` metadata
stays consumer-owned.

The [Python extension](docs/python.md) uses a separate PyO3/maturin package,
preserving the core, CLI, and fuzz dependency boundaries. Immutable Rust documents
and a resolved snapshot expose plain data and separate tag exports. One numeric
conversion seam uses Python integers and `decimal.Decimal`, never float. Parsed
exports use core string accessors; tag traversal shares escaped-pointer encoding
and one reusable path buffer, avoiding an intermediate source-tag map and
per-edge string copies. Snapshot tag entries use an append-only vector until
Python builds the exported dictionary; there is no redundant Rust hash table.
AST-backed and owned-JSON traversals stay separate to avoid allocating an
intermediate JSON tree for parsed exports. They share numeric and pointer
conversion, not a generic conversion framework. Schema admission remains in
Rust; composition admits Python iterables directly into shared module handles,
then clones schema trees lazily during consuming composition. No intermediate
Python-object vector or sequence-only restriction is retained.
The standard mixed Python/Rust package layout keeps one native stub and the
`py.typed` marker beside the extension for both wheel and source installations.
The small public initializer only re-exports that native API; no archive-copy
script or duplicate stub source is maintained.
No PyPI publication, cross-platform wheel support, or consumer construction
is implied.

Local verification: the aggregate Rust/Python gate passes, including eleven Python
boundary cases. Default/all-feature native, fuzz, and binding dependency policies
pass without ignored advisories. The stale fuzz lockfile's core dependency edge
was refreshed without changing package versions.

The same private `cp312-abi3-manylinux_2_34_x86_64` wheel was installed into clean
CPython 3.12.3 and 3.13.15 environments. The source archive was rebuilt and
installed independently on 3.12.3. All three ran outside the checkout and passed
exact-number, tag/default, error/path, formatting, discovery, snapshot-isolation,
and shared-registry probes (40 calls across four worker threads per environment).
Wheel and source-installed packages both contain the native stub and `py.typed`.
A typed iterator consumer passes basedpyright against each distribution path;
the pre-repair wheel rejected that supported call. CI and other platforms are
not claimed by these local results.

The sequence syntax decision is settled: native definitions use separate
`!List { items: ... }` and `!Tuple [ ... ]` constructors. List retains repeated
element constraints and its `min`, `max`, and `unique` facets. Tuple declares
exact positions without those facets; empty tuples have zero positions and
duplicate values are allowed. Bare brackets remain ordinary sequence data,
not an alternative positional-definition spelling.

Tuple defaults belong after the complete definition:
`!Tuple [!Int] = [1]`. Direct scalar position defaults, including grouped
assignments and positional `= unset`, are excluded. This removes the former
inherited-array-preset question rather than adding implicit array editing.
A child uses `!self.Base = [2]` to replace the whole default or
`!self.Base = unset` to remove it without changing positional constraints.
Nested object-member completion remains unchanged. Positional narrowing of
a reference uses `!self.Base & !Tuple [ ... ]`, not reference-plus-bare-brackets.

The explicit-tuple cutover passes `CARGO_BUILD_JOBS=1 ./scripts/check`, including
the Rust, stdio LSP, doctest, documentation, and eleven Python boundary cases.
Frozen native checks cover exact positions, nesting, empty tuples, duplicate
values, whole-default replacement/removal, and rejection of position defaults
and tuple metadata. Existing object-default projection and untouched union
alternatives remain covered. Recursive lowering uses smaller tagged helpers
so the cumulative depth limit produces a structured error on the standard test
thread stack without increasing that stack or reducing the limit.

Actual native CLI and Python-extension probes complete omitted tuple members,
replace whole defaults, accept explicitly supplied values after `unset`, and
allow duplicate tuple values. They reject missing positions, tuple metadata,
grouped position defaults, and duplicate values in a unique List. Formatter
CLI probes retain explicit Tuple syntax, comments, and numeric spelling, and
pass canonical `--check`; ordinary tagged arrays remain data.
Grouped tuple/reference operands and reassociated conjunctions preserve nested
member-default replacement and removal. Regression checks and native/Python
probes cover the repaired grouping boundary; empty Tuple formatting also passes
canonical checking at width 1.
Sibling union alternatives remain independently scoped: a supplied member keeps
the replaced tuple default, while differing omitted-member completions remain
ambiguous. The regression and native/Python probes cover this boundary without
embedding registered base identifiers or restoring replaced defaults.

### Core closeout and delivery

The closeout removes inferred member declarations and inline scalar field defaults.
Scalar data belongs in the record's grouped preset. Root defaults, nested
container presets, and `List.items`/`Map.values` metadata presets remain supported.
Reference-dependent `unset` admission uses bounded schema proof, including
container constants; it does not add filesystem access to syntax parsing.

The behavior-preserving refinement gives seven read-only compiler walks one
borrowed schema-registry lookup, `Compiler::schema_for_reference`. Traversal,
projection, copying, and budget policies remain separate. Mutable projection
retains its identity-owning lookup. Formatter simplification removes a redundant
inherited-default guard while retaining the caller fast path and unchanged
comment/literal framing.

The complete local aggregate gate passed before final documentation/fuzz repairs.
The repaired candidate passed its Rust gate and fresh Python component separately
after a host thread-creation failure; the exact committed candidate then passed
the complete aggregate, dependency-policy, and fuzz gates on GitHub. Coverage
includes 53 native-bundle cases and all eleven Python boundary cases. Actual
native CLI/formatter probes cover retained defaults, scalar/mixed-field rejection,
container narrowing, and canonical output. The six-file MIT native archives pass
installed formatter/LSP contracts on Linux, macOS, and Windows. The clean released
Linux archive also passes the real OMP 18.12.6 RPC/LSP probe with zero model turns
and removes its isolated state. The earlier skill record remains explicitly
dirty-local proof, not the published archive's provenance.
Review repair moves the composition example's scalar data into its record
preset. Separate valid formatter seeds retain grouped bare-tagged container
presets and grouped multiline scalar-root framing. Both seeds reach the actual
formatter, and `scripts/fuzz-smoke 100` passes all four bounded corpus runs.

The private [v0.1.0 release](https://github.com/dprb-work/daml/releases/tag/v0.1.0)
publishes three native archives and checksum sidecars from clean main commit
`a75ae1ef881180c1c684ee0f68710091e23c650f`, after both main workflows passed.
Every archive identifies that commit and includes MIT/dependency notices.
Uploaded archive digests match the verified CI downloads; authenticated retrieval
and checksum verification of the published Linux asset also pass. Repository
visibility remains private; crate/PyPI publication remains disabled.

### Editor and host integrations

The [integration packages](integrations/README.md) follow the published core:
an installable VS Code/Cursor VSIX and mergeable OMP/DSH registrations. They use
the existing syntax/formatting LSP, not schema-aware editor features. The VSIX
uses `vscode-languageclient`, a distinct language ID, a machine-scoped executable
setting, workspace trust, and generated bundled dependency notices.

Native DSH reuses Soul's canonical client and workspace authorization boundary.
Its generic formatting preview returns `{ uri, version, edits }` without writes;
`version` is the opaque filesystem freshness token. A private LSP document
generation guard rejects concurrent same-document synchronization. The real-server
regression fails before that guard and passes after it.
Cold-start overlap has a separate real-server regression: reserve each document
generation before awaiting notification writes, so duplicate opens cannot pair
earlier text with a newer filesystem token. It fails before reservation and
passes afterward. The installed VSIX also retains the standard language client's
executable Unix termination helper; a lingering-parent restart case fails without
that asset and passes with it. Native smoke cleanup runs even if disposal fails.

Actual verification covers installed VS Code 1.105.1 diagnostics/clearing,
Unicode edit application, idempotence and invalid-input preservation; a rendered
editor session also exercises the error-to-valid-to-formatted transition.
Trusted/restricted Electron cases preserve the executable-setting boundary and
prevent untrusted server launch while retaining highlighting. Actual TextMate
checks cover string/Regex boundaries; the escaped-backslash comment-scope
regression fails before the grammar fix and passes afterward.
The packaged registration passes real OMP 18.12.6 RPC. Native DSH passes its
real Cordis/tools/local-filesystem/workspace-adapter smoke and lifecycle/snapshot
regressions. Strict client builds, npm audits, and Soul's full `pnpm run check`
pass. Cursor compatibility is
declared through the VS Code API; Cursor itself was not launched. No active host
installation/configuration is changed.


### Standalone browser SDK

The [private browser SDK](bindings/browser/README.md) implements
[DAML issue 16](https://github.com/dprb-work/daml/issues/16), the standalone
browser-authoring prerequisite for [Olaf issue 387](https://github.com/dprb-work/Olaf/issues/387).
Olaf adoption remains a separate change; no Olaf code, hosted CI, active host
installation, or registry publication is part of this delivery.

`Bundle::from_sources` supplies bounded logical resources to the existing native
compiler. Canonical semantic writing accepts edited exact values and preserves
assertion identity through visible aliases, rejecting private collisions and
constrained-alias substitutions. WASM classes expose those core operations,
original source, owned snapshots, and input/default provenance.

Consolidation review retains one compiler for filesystem and supplied resources,
one Rust language engine across native/browser clients, and the existing shared
license-policy producer. The browser bridge converts only at its JS boundary;
resolved values and origins are exported in one traversal, without an
intermediate owned Rust snapshot. Simplification removes up-front sparse-list
capacity allocation, repeated pointer-prefix copies, alias candidate sorting,
and unnecessary scalar conversions. Independent core and binding reviews approve
the repaired source; neither review substitutes for runtime evidence.

The ownership refinement borrows selected tag names from their input/registry
and borrows JS record handles during admission; neither needs a copied owner.
`scripts/cargo-notices.py` now owns notice generation for both packagers,
including the tool pin, policy translation, template, and temporary config.
This removes the browser's import of native-packager internals and its duplicate
notice orchestration. Both native and WASM legal payloads match their previous
producers byte-for-byte. Source-preserving reflow, semantic writing, and JS
transport retain separate contracts rather than becoming a generic visitor.

Focused local evidence covers 142 Rust cases across `browser_resources`,
`semantic_write`, `native_bundles`, `default_tag_annotations`, `defaults`, and
`schema_loading`; core and WASM-binding Clippy with warnings denied; generated
TypeScript consumer checking; and the real native parity executable.
Playwright drives release WASM in Google Chrome for Testing 153.0.8010.12,
including the extracted private tarball, with zero requests after static preload.
Rendered package inspection also confirms the documented edit/write/re-admit
path, snapshot mutation isolation, and exact Unicode diagnostic coordinates.

Local packaging includes canonical MIT/dependency notices, source/output/legal
hashes, and a SHA-256 sidecar. An injected unrecorded `dist` file is rejected
before packaging. Only a clean committed build may be promoted as a release;
building and verifying a private local tarball does not publish it.
The [verifier guide](bindings/browser/verify/README.md) owns repeatable commands.
CI optimization remains blocked here because another agent owns it.

### Conversion agenda

Treat configuration conversion and definition conversion as separate contracts:

- Configuration: YAML/JSON to DAML and DAML to YAML/JSON. Reuse a maintained
  YAML parser/emitter rather than owning YAML grammar. Decide the admitted
  YAML profile first: duplicate keys, non-string keys, aliases/cycles, custom
  tags, non-finite numbers, and multiple documents cannot be silently coerced.
  Define the explicit tag-loss policy for plain exports and distinguish
  syntax-only export from completed/validated export. DAML's unquoted key
  grammar cannot represent every arbitrary JSON/YAML string key.
- Definitions: JSON Schema, including JSON Schema serialized as YAML, to native
  DAML definitions and back where representable. Report unsupported keywords,
  dialects, recursive graphs, regex differences, and consumer annotations;
  never silently discard constraints. Bundle imports, explicit assertions,
  default replacement, and completion are not fully expressible as standard
  JSON Schema. Keep this separate from merely serializing an interchange module.
- Source fidelity is not semantic conversion: comments, scalar styles, anchors,
  source spans, and numeric spelling need an explicit preservation contract.
  Reuse existing tooling if source-preserving YAML editing is needed.

No converter is implemented by this agenda. Native `contains`, conditionals,
general negation, and package/network retrieval remain outside this delivery.

### Helm assessment

The narrow integration point is completed plain values before Helm renders:
serialize them to a values file, or use selected JSON overrides through
[`--set-json`](https://helm.sh/docs/helm/helm_install#synopsis).
Helm retains its own [values precedence and merging](https://helm.sh/docs/chart_template_guide/values_files/).
DAML tags and provenance do not survive that plain-data boundary.

A chart's [`values.schema.json`](https://helm.sh/docs/topics/charts/#schema-files)
is the schema interchange seam, not a native DAML definition. Helm's documented
example uses Draft-07; DAML's admitted interchange dialect is Draft 2020-12.
Do not infer dialect compatibility or skip chart validation. JSON Schema
[`default`](https://json-schema.org/understanding-json-schema/reference/annotations#default)
does not perform DAML completion.

[Post-renderers](https://helm.sh/docs/topics/advanced/#post-rendering) receive
already rendered manifests, too late to convert values. A future CLI wrapper
could prepare values and invoke Helm; no plugin, chart mutation, deployment,
or Helm-specific runtime is needed here. Helm 3/4 behavior must be selected
explicitly if such an integration is later accepted.
