## 1. Scope decision and independent checks

- [x] 1.1 Obtain human acceptance of the reusable Rust API plus CLI caller, snapshot-relative operations and explicit comment ownership; record the decision separately from this proposal.
- [x] 1.2 Review concrete public API and finite operation/work/output limits against existing document, renderer and schema owners.
- [x] 1.3 Independently author and freeze source-editing acceptance checks, including real source preservation and stale/conflict failures.

## 2. Implementation

- [x] 2.1 Implement snapshot-relative path admission, conflict detection and bounded source patching through existing parser spans.
- [x] 2.2 Reuse semantic rendering and final parse/resolve admission without materializing unrelated defaults or losing binding identity.
- [x] 2.3 Add `daml edit --edits FILE` with bounded DAML batches, existing context selection, final validation and authored-source stdout; keep all inputs read-only.
- [ ] 2.4 Challenge the composed patch/application/CLI path; examine consolidation and simplification without adding a second grammar or evaluator.
- [x] 2.5 Reconcile source-editing and CLI documentation separately from formatting and canonical writing.

## 3. Verification and acceptance

- [x] 3.1 Exercise the actual edit/apply/reparse/resolve journey, focused boundaries and unchanged existing consumers.
- [ ] 3.2 Run repository-required checks and strict OpenSpec validation on the integrated candidate.
- [ ] 3.3 Complete independent whole-change review and disposition findings.
- [ ] 3.4 Obtain human result acceptance; merge, release and caller/host activation remain separate decisions.

## Current state

Issue #34: the owner accepted core operation/comment/snapshot rules and selected CLI as the first caller. Core and CLI implementation, independently owned focused checks and a composed CLI smoke have passed; integrated challenge, aggregate verification and final independent review remain pending. No result acceptance, merge, release or host activation is claimed.
