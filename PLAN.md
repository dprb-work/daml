# DAML implementation plan

Status: syntax, structural schema validation, default completion, hardening,
reflow formatting, the minimal stdio LSP, and binary distribution proof are
implemented. [Syntax](docs/syntax.md) and [schemas](docs/schemas.md) own current
behavior. Key/tag alphabets and native schema authoring remain open.

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

Unquoted, spaceless keys are confirmed. Final key/tag character sets and whether
they align are deferred. They are local lexical policies, not prerequisites for
starting the parser. The current scanner bounds both by structural delimiters
and whitespace; it does not freeze numeric-leading, dotted, slashed, or Unicode
identifier rules.

Names remain literal. Dots/slashes never imply nesting, path traversal,
references, or file access; braces establish nesting. Schema lookup, formatting,
and the language server must use actual names/tree ownership rather than infer
structure from punctuation. Do not expose competing grammar modes.

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
  do not grow a YAML compatibility mode. Final key/tag alphabets may remain
  provisional behind the lexical policy.
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
bounds. Only the final key/tag alphabets are intentionally left provisional.

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

## Accepted native-authoring direction, not implemented

Future bundle authoring uses keyed bundle roots, where each key names a
namespace and each value gives its bundle root. Each tag walks directly to one
file containing that definition. Imports have private schema-local scopes:
transitive imports do not expose names to roots, parents, or siblings, though a
root may import a dependency under another alias. A tag reference selects a
definition directly; exact spellings, metadata placement, scope granularity,
and loading identity remain open. Value-shaped scalar narrowing is distinct
from schema definitions, and compatibility is structural.

Authoring reuse has one base. Constraint, requiredness, and default are
independent override components. A replacement constraint expression is atomic,
not a recursive keyword patch. Derived definitions may change all three without
waiving structural compatibility at a use site. Existing object-default/data
merging follows the [schema guide](docs/schemas.md#exact-completion-rules);
native default-removal syntax remains open. Array narrowing remains
**undecided**; the question about positional patterns was cancelled, so no array
behavior is selected.

Changes must satisfy these maintenance criteria:

- Own custom responsibilities only for required behavior that sufficient
  libraries or platform facilities do not provide.
- Count adapters, integration, operational assumptions, upgrades, and recovery
  as maintenance costs. Material scope growth needs an explicit owner decision.
- Every code delivery includes consolidation followed by simplification across
  the normal end-to-end ownership and validation flow; justified no-change
  conclusions are valid.
- Admission establishes local shape and invariants. Revalidation needs a
  distinct reason, such as mutation or a new combined-resource boundary.
- Reject knowable violations before dependent effects, without disguising
  fallbacks. Preserve independently evolving boundaries and justified safeguards.

The structural cutover removed nominal membership/custom validators in favor of
the existing JSON Schema engine. Post-merge refinement keeps local shape checks
at admission, preserves composition's aggregate/reference checks, and removes
obsolete identifier parameterization and per-byte temporary strings.

The missing native authoring/import syntax blocks frontend work, not the
structural-validation cutover.

## Milestone 4 — harden and ship the standalone library

**Implemented hardening baseline:** retained parse/schema/resolve fuzz targets,
bounded AddressSanitizer campaigns, licensed YAML/DAML corpus cases,
cumulative-budget regressions, and release time/process-RSS measurements.
See [the measured evidence](docs/hardening-baseline.md) and
[reproduction commands](docs/hardening.md). Publication remains blocked on
an explicit project-license and packaging decision; `publish = false` remains.

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
  code, fixtures, and dependencies. Keep `publish = false` until publication and
  DAML's own distribution license are explicitly selected.
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
- Schema-folder root selection and the base for relative schema paths remain
  unresolved. Discuss them after the minimal LSP, during final sharpening;
  this slice must not infer discovery rules from cwd, workspace, or document paths.

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

Serde integration, Python bindings/wheels, and a full command-line product remain
separately scoped. Their earlier illustrative APIs are not frozen commitments.

The standalone library now has a retained hardening baseline, reflow formatter,
and minimal stdio language server. The sequence below puts binary-distribution
proof before final sharpening and consolidation. Schema resolution remains
independent of application construction and execution.

The [edge-case record](docs/edge-cases.md) retains 45 exact runtime probes.
Triple-double opening/closing newline framing and mandatory dedent are confirmed.
The record separates those decisions from current observations and unresolved
alphabet/edge-case choices; formatter work does not silently settle them.

The [formatter reuse assessment](docs/formatter-reuse.md) records the selection
of `pretty` after proving source-sensitive literal and comment fixtures. Host
integration remains separate; no formatter platform or style framework is added.

## Next sequence

Binary distribution proof is merged, and structural consolidation and
simplification are complete. The next session starts by sharpening the deferred
native authoring and import syntax before starting Python bindings. Array
narrowing remains undecided after the cancelled question. Editor integrations
remain deferred.
