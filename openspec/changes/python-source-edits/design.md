## Accepted boundary

The owner approved the bounded Python feature after the proposal to expose the existing editor as one-shot replace/insert/remove, returning a new document, with existing schema validation, source/exact-value/default-omission guarantees and no automatic writes or OMP changes. [Issue #54](https://github.com/dprb-work/daml/issues/54) records the decision. This authorizes implementation, not result acceptance, publication, installation or host activation.

## API and ownership

`Document.edit(edits, line_width=80)` uses the parse-only core entry point. `Schemas.edit(document, edits, expected=None, line_width=80)` uses the contextual core entry point. The latter validates the final edited document under existing assertion and optional expected-contract semantics; it does not initially resolve the original document, so a bad supplied value can be repaired. An immutable registry must already admit the desired visible assertion names. No implicit bundle reopening or schema fetch is added.

An operation is a strict dictionary containing `op`, `path`, and a normal semantic `value` for replace/insert. Remove forbids `value`, even `None`. Only list/tuple batches are accepted, not arbitrary iterables or generators. Existing semantic node/tag records retain exact numeric lexemes and intrinsic temporal/cron kinds; ordinary Python values are not silently coerced. Stage callback operations such as copy/expect keep their separate semantics and are not source operations.

The binding admits the full batch while holding the GIL into owned Rust paths and semantic values. One shared `semantic::Budget` covers metadata, expected name, paths and all values. Actual sequence length is checked against `SourceEditLimits::default().max_operations` before allocating request storage or touching elements. Other core/parser/writer/resolver bounds remain authoritative; no new configurable limits class is introduced.

Frozen document/registry references remain alive while `py.detach` runs the core preparation and application. Only the owned resulting Rust `Document` crosses back into a new Python `Document`. No borrowed patch object survives across Python calls. The original document/tree is not cloned or converted into a semantic tree. Python input mutation after admission cannot change the owned batch. The API writes no files and makes no stale-disk claim; the original frozen document is its exact source snapshot.

## Errors and typing

Core failures become public `SourceEditError(ValueError)` with `kind`, `path` and `operations`. Pointer/missing-path/invalid-target errors carry their path; conflicts carry the original zero-based operation pair; other metadata is `None`. Wrapped parser, semantic-renderer and resolver failures retain existing public typed exceptions as `__cause__`. Malformed binding/semantic transport and its aggregate budget use existing `SemanticError` conventions. PyO3 argument-conversion failures retain existing built-in exception behavior.

Existing exception translation in the native module remains the owner of public causes and attributes. A private source-edit bridge module owns only new request admission and the one-shot core call. The stub reuses the existing replacement-record shape and adds only insert/remove union members plus method/error typing. No runtime operation classes or compatibility aliases are required.

## Alternatives not selected

- Canonical `write` cannot preserve comments/spelling it does not receive; it keeps its existing semantic-writing contract.
- A CLI subprocess cannot directly use an in-memory Python registry and adds process/file coordination.
- A persistent Python patch object would require new cross-call lifetime and snapshot ownership without a current need.
- A second Python parser, patcher or evaluator would duplicate the established owners.

## Evidence and delivery

Independent tests own binding-level source fidelity, original-index semantics, read-only input/value behavior, exact/intrinsic/binding round trips, checked default omission/repair/rejection, strict shapes/cycles/Unicode/budgets, and public error metadata/causes. A freshly rebuilt baseline demonstrates the missing method through an actual Python call before production edits. Main runs the finished binding suite, real Python schema-derived workflows, aggregate repository gate and strict OpenSpec validation. Independent integrated challenge and exact-head whole-change review remain separate from owner result acceptance.
