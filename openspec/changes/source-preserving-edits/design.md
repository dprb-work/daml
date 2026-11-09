## Status and decision owner

Proposal for issue #34; detailed scope is pending the user's decision. CLI #32 is integrated. LSP #33 is human-accepted and undergoing integration. This document does not authorize implementation, merge, publication or host activation.

## Existing owners and evidence

`Document` owns original UTF-8 source and an ordered parsed tree (`src/model.rs`). `Node.span` covers its tag and value; mapping entries retain key and value spans. `Document::source` and checked `slice` expose the original bytes. The parser admits complete values and bounded source; it is not an editor-specific repaired grammar.

`SemanticNode` already represents exact numbers, owned temporal/cron values and optional stable schema bindings (`src/semantic.rs`). Existing canonical rendering owns representability checks. It intentionally does not preserve source presentation (`docs/conversion.md`). Source editing must splice only newly rendered regions rather than serializing the entire resolved tree.

## Proposed boundaries

The first caller surface is the portable Rust library, not an implicitly writing CLI/editor command. A batch contains explicit replace/insert/remove operations. Insert/replace use existing semantic values; untouched nodes are not converted or cloned into a second semantic tree. Only newly supplied values need rendering. Final parsing and, when requested, existing schema resolution own admission.

All paths are original-snapshot paths. Distinct array indices therefore keep their original meanings within one batch. Apply disjoint byte edits from the end of the source after proving their ranges and combined size. This avoids sequential reindexing and partial success. Exact public Rust names and numeric batch limits are implementation proposals to review before freezing checks; the contract requires finite aggregate admission compatible with existing parser limits.

Prepared edits borrow or otherwise identify their original source. Application compares the current source exactly before using stored byte ranges. A borrowed-source representation can avoid copying the original snapshot and needs no hash dependency. Editor versions may be an additional caller guard; they are not a substitute for core snapshot identity. This feature does not add an LSP edit command or persistence layer.

## Comment and insertion policy

Use syntactic containment, not indentation or guessed human intent. Replacement intentionally replaces the complete node, including internal comments. Removal of a mapping entry removes key through value; removal of an array item removes its node. Comments outside those ranges stay exactly where they were, even if this leaves an orphan comment or extra blank line. Reformatting is an explicit separate operation.

New map fields append before the closing delimiter. Array inserts select an original before-index or append at the original length. Generated separators must keep a preceding line comment from consuming the new value and preserve all original trivia. Structural newlines use the parent container's first physical newline style, otherwise LF. The renderer remains responsible for replacement block-string semantics; newline-style conversion must not change decoded values.

Root replacement is allowed; root insert/remove are errors. Replacing a tagged node explicitly replaces its whole assertion and value; untouched tags are never reconstructed or weakened. Existing `Schemas` source representability rules refuse a private binding with no usable spelling.

## Alternatives not selected

- Whole-document canonical writing is simpler but violates source fidelity and can materialize defaults.
- A second concrete-syntax parser duplicates grammar ownership; derive trivia boundaries from the existing source/parser owners instead.
- Sequentially applying paths makes array operations order-dependent; snapshot-relative operations make batches reproducible.
- Inferred leading-comment attachment can delete unrelated commentary; preserving outside-span comments has a precise byte contract.
- Automatic file writes and implicit schema discovery add effects and policy that are unnecessary for the requested core editing capability.

## Verification and recovery

An independent check owner must freeze consumer-visible cases before implementation. Cover actual patch/application/reparse/resolve, original bytes, exact values, comment ownership, empty/tagged containers, escaped pointers, original array indices, mixed newlines, block strings, Unicode, stale snapshots, conflicts and resource bounds. Retain existing formatting/writing/CLI/LSP tests unchanged.

No migration or rollback of persistent state is needed: the API returns a new document and does not overwrite files. Existing formatting and semantic writing stay available with their existing contracts.
