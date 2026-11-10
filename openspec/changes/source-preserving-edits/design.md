## Status and decision owner

The owner [accepted the core rules and selected CLI as the first caller](https://github.com/dprb-work/daml/pull/41#issuecomment-6777065470) for issue #34. Existing CLI and LSP operations remain available. This decision authorizes implementation, not result acceptance, merge, publication or host activation.

## Existing owners and evidence

`Document` owns original UTF-8 source and an ordered parsed tree (`src/model.rs`). `Node.span` covers its tag and value; mapping entries retain key and value spans. `Document::source` and checked `slice` expose the original bytes. The parser admits complete values and bounded source; it is not an editor-specific repaired grammar.

`SemanticNode` already represents exact numbers, owned temporal/cron values and optional stable schema bindings (`src/semantic.rs`). Existing canonical rendering owns representability checks. It intentionally does not preserve source presentation (`docs/conversion.md`). Source editing must splice only newly rendered regions rather than serializing the entire resolved tree.

## Implementation boundaries

The portable Rust library owns a batch of explicit replace/insert/remove operations. Insert/replace use existing semantic values; untouched nodes are not converted or cloned into a second semantic tree. Only newly supplied values need rendering. Final parsing and, when requested, existing schema resolution own admission. `daml edit` is the first caller and returns authored source on stdout; it does not write files.

All paths are original-snapshot paths. Distinct array indices therefore keep their original meanings within one batch. Apply disjoint byte edits after proving their ranges and combined size. This avoids sequential reindexing and partial success. `SourceEdit`, `SourceChange`, `SourcePatch` and `SourceEditLimits` expose the bounded plan/application boundary. Default admission allows 1,024 operations, 16 MiB cumulative work, 8 MiB output and existing parser limits; callers may choose stricter explicit limits. Work accounting includes source inspection, pointers/traversal and supplied/rendered values.

Prepared edits borrow or otherwise identify their original source. Application compares the current source exactly before using stored byte ranges. A borrowed-source representation can avoid copying the original snapshot and needs no hash dependency. Editor versions may be an additional caller guard; they are not a substitute for core snapshot identity. This feature does not add an LSP edit command or persistence layer.

### CLI admission

`daml edit --edits FILE [--bundle DIR | --module FILE ...] [--expected NAME] [CONFIG|-]` reuses the existing context and configuration-input conventions. The edits file is a bounded, untagged DAML sequence of untagged operation maps. Each map contains untagged string `op` and `path`; replace/insert require a normal DAML `value`, while remove forbids it. Unknown fields and operations fail. The edit source has the normal 8 MiB input bound, and the core operation/work/output limits also apply.

`--edits` is required exactly once for edit and forbidden on other operations; `--edits -` is rejected because stdin belongs to the configuration. Existing `--format` and `--path` remain resolve-only and explain-only. Missing/malformed flags and I/O failures exit 2; batch, path, value, schema and resource failures exit 1 with no successful stdout.

The CLI applies the parse-only core patch before compiling its normal schema context against the edited document. This admits newly introduced bundle assertions and permits repairing syntactically valid but schema-invalid values. It then resolves the final document for validation and returns that authored source, not the resolved tree. Defaults remain omitted unless an edit explicitly supplies them. Existing file-relative bundle discovery uses the configuration path; stdin still requires explicit context.

### Public plan/application boundary

`Document::prepare_source_edits` admits paths and supplied values without schema context. `Schemas::prepare_source_edits` also retains the selected registry and optional expected contract for contextual rendering and final validation. Prepared patches borrow the original document and expose immutable `SourceChange` byte regions. `SourcePatch::apply` first compares the exact current source, then builds, reparses and optionally resolves the complete candidate before returning a new `Document`. Preparation admits a plan; successful application establishes final document admission.

`SourceEditError` keeps distinct pointer, missing-path, invalid-target, conflict, budget and stale-source categories and retains parser, semantic-renderer and resolver causes. No private error type escapes the public boundary.

## Comment and insertion policy

Use syntactic containment, not indentation or guessed human intent. Replacement intentionally replaces the complete node, including internal comments. Removal of a mapping entry removes key through value; removal of an array item removes its node. Comments outside those ranges stay exactly where they were, even if this leaves an orphan comment or extra blank line. Reformatting is an explicit separate operation.

New map fields append before the closing delimiter. Array inserts select an original before-index or append at the original length. Generated separators must keep a preceding line comment from consuming the new value and preserve all original trivia. Structural newlines use the parent container's first physical newline style, otherwise LF. The renderer remains responsible for replacement block-string semantics; newline-style conversion must not change decoded values.

Root replacement is allowed; root insert/remove are errors. Replacing a tagged node explicitly replaces its whole assertion and value; untouched tags are never reconstructed or weakened. Existing `Schemas` source representability rules refuse a private binding with no usable spelling.

## Alternatives not selected

- Whole-document canonical writing is simpler but violates source fidelity and can materialize defaults.
- A second concrete-syntax parser duplicates grammar ownership; derive trivia boundaries from the existing source/parser owners instead.
- Sequentially applying paths makes array operations order-dependent; snapshot-relative operations make batches reproducible.
- Inferred leading-comment attachment can delete unrelated commentary; preserving outside-span comments has a precise byte contract.
- Automatic file writes add an unnecessary effect; the CLI returns source on stdout and retains its existing context-selection behavior.

## Verification and recovery

An independent check owner must freeze consumer-visible cases before implementation. Cover actual patch/application/reparse/resolve, original bytes, exact values, comment ownership, empty/tagged containers, escaped pointers, original array indices, mixed newlines, block strings, Unicode, stale snapshots, conflicts and resource bounds. Retain existing formatting/writing/CLI/LSP tests unchanged.

No migration or rollback of persistent state is needed: the API returns a new document and does not overwrite files. Existing formatting and semantic writing stay available with their existing contracts.
