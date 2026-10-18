# DAML implementation plan

Status: syntax parsing and deep assertion/default resolution are implemented
(milestones 1–3). [Syntax](docs/syntax.md) and [schemas](docs/schemas.md) own their
public contracts. Retained fuzzing, measurements, formatter, and LSP work remain
planned. Final key/tag alphabets and whether they align remain deferred.

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

## Milestone 3 — deep assertions and independently owned schema modules

**Result:** a document's nested assertions are actually enforced without editing
a central union whenever a new module supplies a type.

The implemented schema/default contract is in [docs/schemas.md](docs/schemas.md).
It uses JSON Schema 2020-12 constraints serialized as JSON or DAML, with explicit
tag/slot keywords and an offline, tag-aware validation engine. Consumers translate
their own formats into native `SchemaModule` values; DAML has no vendor importer.

- Add a separate validation pass over the parsed document. Unknown tags survive
  syntax parsing but fail validation unless explicitly supplied by the caller.
- Keep tag resolution separate from application-object construction. Validators
  do not execute callables, import modules from tag strings, resolve templates,
  read environment variables, or access the network.
- Let modules export named schemas and supported extension contracts. Compose
  the selected modules explicitly; duplicate/conflicting tag names are errors,
  not last-writer-wins overrides.
- Require both the containing field contract and the tagged node contract to
  hold. A tag cannot cast away a field restriction. Visit asserted descendants
  even beneath otherwise open/untyped containers.
- Start with the record/scalar/sequence and extension-contract capabilities the
  fixtures require. Do not invent a second general-purpose schema language or
  require a full JSON Schema implementation without a consumer need.
- Keep the validation schema model declarative and language-neutral so a later
  Python adapter can describe types without Python callbacks during every field
  check. Do not implement PyO3 or Python type inspection in this milestone.

**Gate:** two independently defined modules compose; a third module's compatible
implementation validates without changing the host schema's discriminator union.
Nested invalid fields, unknown tags, incompatible field/tag contracts, and
composition collisions produce structured paths plus original source spans.
Tagged scalars, lists, mappings, and empty mappings are all exercised. A real
consumer calls parse then resolve, including defaults; syntax, schema-loading,
and resolution failures stay separate. `./scripts/check` passes.

An effective name-to-schema lookup is still necessary. The decoupling is in
schema ownership and extension contracts, not the absence of an index.

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

- Reuse the parser, validator, and formatter. Start with document open/change/
  close, syntax diagnostics, and document formatting; add schema diagnostics
  when the client supplies the relevant schema modules.
- Convert original byte spans to the negotiated LSP position encoding correctly,
  including multibyte characters in values/comments.
- Associate results with document versions so stale work does not replace newer
  diagnostics or format a different revision. Keep workspace schema discovery
  separate from executing application code.

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

The formatter follows the standalone library; the LSP remains a later milestone,
not a hidden library prerequisite. They share source information rather than
introducing another grammar or changing string rules.

Serde integration, Python bindings/wheels, and a full command-line product remain
separately scoped. Their earlier illustrative APIs are not frozen commitments.

The standalone library has a retained hardening baseline and a reflow formatter.
The next planned implementation milestone is the language server; final key/tag
alphabets remain a separate language decision. Schema resolution enforces
independently supplied assertions and completes explicit defaults without
constructing application objects.

The [edge-case record](docs/edge-cases.md) retains 45 exact runtime probes.
Triple-double opening/closing newline framing and mandatory dedent are confirmed.
The record separates those decisions from current observations and unresolved
alphabet/edge-case choices; formatter work does not silently settle them.

The [formatter reuse assessment](docs/formatter-reuse.md) records the selection
of `pretty` after proving source-sensitive literal and comment fixtures. Host
integration remains separate; no formatter platform or style framework is added.
