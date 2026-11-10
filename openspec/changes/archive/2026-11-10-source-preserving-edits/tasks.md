## 1. Scope decision and independent checks

- [x] 1.1 Obtain human acceptance of the reusable Rust API plus CLI caller, snapshot-relative operations and explicit comment ownership; record the decision separately from this proposal.
- [x] 1.2 Review concrete public API and finite operation/work/output limits against existing document, renderer and schema owners.
- [x] 1.3 Independently author and freeze source-editing acceptance checks, including real source preservation and stale/conflict failures.

## 2. Implementation

- [x] 2.1 Implement snapshot-relative path admission, conflict detection and bounded source patching through existing parser spans.
- [x] 2.2 Reuse semantic rendering and final parse/resolve admission without materializing unrelated defaults or losing binding identity.
- [x] 2.3 Add `daml edit --edits FILE` with bounded DAML batches, existing context selection, final validation and authored-source stdout; keep all inputs read-only.
- [x] 2.4 Challenge the composed patch/application/CLI path; examine consolidation and simplification without adding a second grammar or evaluator.
- [x] 2.5 Reconcile source-editing and CLI documentation separately from formatting and canonical writing.

## 3. Verification and acceptance

- [x] 3.1 Exercise the actual edit/apply/reparse/resolve journey, focused boundaries and unchanged existing consumers.
- [x] 3.2 Run repository-required checks and strict OpenSpec validation on the integrated candidate.
- [x] 3.3 Complete independent whole-change review and disposition findings.
- [x] 3.4 Obtain human result acceptance; merge, release and caller/host activation remain separate decisions.

## Current state

Issue #34: the owner accepted result `471e444bbac6cd599a1608763ceb911b2c7f826a` in [the acceptance record](https://github.com/dprb-work/daml/pull/41#issuecomment-6778231345) and separately authorized its [merge as `335a9fc14e91e3c581e1b4a6ad9da1ebec155557`](https://github.com/dprb-work/daml/pull/41#issuecomment-6778672293). Implementation, independent checks/review, composed smoke, local aggregate and required hosted CI passed. No release publication, CLI installation or host activation was performed.
