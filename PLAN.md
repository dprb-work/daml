# DAML implementation plan

Status: proposed implementation sequence. User-confirmed requirements are below;
new grammar and API choices are proposals until the first contract gate closes.
This change writes the plan and reuse assessment only, not a parser.

## Target result

A standalone safe-Rust library that parses DAML into a source-backed document,
preserves deeply nested tags, and validates their assertions against independently
composed schema modules. A real consumer can inspect values, tags, and source
locations without depending on Olaf or a global discriminator union.

Ship complete consumer-visible behavior in each implementation slice. A lexer,
AST skeleton, forwarding API, or passing empty test suite is not a milestone.

## User-confirmed requirements

- Explicit mapping/list structure inspired by KYAML, without commas.
- `!tags` on nested nodes; an empty tagged mapping can omit `{}`.
- Ordinary `"..."` strings.
- `'''...'''` for one logical line wrapped over multiple source lines: trim each
  source line, discard blank lines, and join remaining lines with one space.
- `"""..."""` for literal multiline text, with **mandatory dedent**. Preserve
  relative indentation and internal line breaks. No optional dedent flag.
- Backslash immediately before a physical newline can explicitly join literal
  string lines; joining inserts no space.
- No YAML `|`, `|-`, `|+`, `>`, `>-`, or `>+` string variants.
- Rust implementation, reusable independently of Olaf. Keep later Python
  integration possible without embedding Python into the core.
- Reuse or port existing code when its technical fit and license justify it.

DAML is its own grammar, not a YAML/KYAML subset. The
[reuse assessment](docs/parser-reuse.md) explains the mismatches, pinned sources,
and license lineage.

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

- Write a compact grammar and a fixture matrix with exact decoded values or
  structured errors. Carry the acceptance specimen and dedent cases into it.
- Mark each YAML-derived fixture as unchanged, adapted, or intentionally rejected.
  Record the original upstream case ID and selected immutable corpus revision.
- Settle the proposals in the following table; do not grow a YAML compatibility
  mode or multiple competing policies while implementing.
- Use the same hard fixtures to assess a candidate reuse seam. Prefer a small
  native scanner/direct parser unless a targeted adaptation or helper such as
  Winnow demonstrably simplifies the implementation. Do not build two complete
  parsers or reject a useful dependency on principle.
- Before copying or porting code/tests, record upstream revision, copied paths,
  destinations, local changes, and required license/copyright notices. Do not
  import a full parser merely to avoid attributing a few useful routines.

| Decision to close | Proposed initial behavior |
|---|---|
| Entry boundaries | Newlines separate mapping/list entries outside strings; blank lines and comments are allowed. Multiple same-line entries without separators are errors. Single-entry inline collections remain valid. |
| Empty tagged mapping | A tag with no payload before an entry boundary or closing delimiter means `{}`. Make next-line payload handling explicit; do not inherit YAML's empty-scalar interpretation. |
| Scalars and keys | String keys only, bare safe identifiers or quoted keys. Reject duplicate decoded keys. Values use quoted strings, lowercase `true`/`false`/`null`, and a deliberately specified decimal-number grammar; no implicit dates, booleans such as `NO`, NaN, or Infinity. Preserve numeric precision/range intentionally. |
| Ordinary strings | A closed escape vocabulary, initially JSON-style; reject invalid/truncated escapes and invalid Unicode scalars. Do not inherit YAML's full escape/folding vocabulary accidentally. |
| Triple-string boundaries | Opening delimiter ends its line; closing delimiter occupies its own line. Boundary newlines are syntax, but deliberate additional blank lines in literal content are data. No implicit trailing-newline stripping beyond those boundaries. |
| Dedent whitespace and escapes | Freeze spaces/tabs, blank-line whitespace, escaped delimiters, literal backslashes, and trailing whitespace after continuation markers. Dedent remains mandatory under one rule, not configurable. |
| Input and excluded YAML | One UTF-8 document, explicit empty document values, defined line-ending policy. Reject anchors, aliases, directives, merge-key semantics, block collections, complex keys, and unsupported string forms rather than silently accepting them. |
| Resource policy | Choose documented input, scalar, node, and nesting bounds before accepting untrusted input; test each boundary and one-over. Do not rely on stack overflow or allocation failure as validation. |

**Gate:** every confirmed spelling has an expected outcome, every proposed choice
above has one selected rule, imported material has its provenance, and there is
one chosen production parsing path. The complete YAML specification is a source,
not DAML's acceptance oracle.

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
consumer calls parse then validate; parser errors and validation errors stay
separate. `./scripts/check` passes.

An effective name-to-schema lookup is still necessary. The decoupling is in
schema ownership and extension contracts, not the absence of an index.

## Milestone 4 — harden and ship the standalone library

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

**Gate:** all accepted behavior is implemented, assertions work in an independent
consumer, no known correctness/security defects remain, bounded fuzz findings
are resolved, and documented commands work. Coverage is assessed by grammar and
failure behavior, not a decorative percentage or test count.

## Boundaries and implementation discipline

Keep one crate and private implementation modules until real boundaries justify
another package. Use safe synchronous Rust, small concrete functions/types,
explicit ownership, and structured errors. Add a dependency only for a concrete
job and apply the researched dependency-policy baseline when doing so. A crate's
unsafe prohibition is not evidence that transitive code contains no unsafe.

Do not silently substitute YAML preprocessing for parsing DAML, implement every
YAML feature, or lose tags by converting immediately to JSON/dictionaries.
Do not couple the grammar or schema owner to Olaf's `__type__`/`__fn__` wire
format, callable runtime, parameter templates, or execution lifecycle.

Formatting, Serde integration, Python bindings/wheels, editor/LSP support, and a
full command-line product are later scopes, not hidden prerequisites for this
parser result. Retaining source and declarative schemas keeps those paths open;
it does not commit us to their earlier illustrative APIs.

The next implementation action is Milestone 1's grammar/fixture contract,
including the mandatory dedent cases, followed by the complete parsing slice.
