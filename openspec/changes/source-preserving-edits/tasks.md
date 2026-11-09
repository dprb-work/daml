## 1. Scope decision and independent checks

- [ ] 1.1 Obtain human acceptance of the proposed Rust-only caller surface, snapshot-relative operations and explicit comment ownership; record the decision separately from this proposal.
- [ ] 1.2 Review concrete public API and finite operation/work/output limits against existing document, renderer and schema owners.
- [ ] 1.3 Independently author and freeze source-editing acceptance checks, including real source preservation and stale/conflict failures.

## 2. Implementation

- [ ] 2.1 Implement snapshot-relative path admission, conflict detection and bounded source patching through existing parser spans.
- [ ] 2.2 Reuse semantic rendering and final parse/resolve admission without materializing unrelated defaults or losing binding identity.
- [ ] 2.3 Challenge the composed patch/application path; examine consolidation and simplification without adding a second grammar or evaluator.
- [ ] 2.4 Reconcile source-editing documentation separately from formatting and canonical writing.

## 3. Verification and acceptance

- [ ] 3.1 Exercise the actual edit/apply/reparse/resolve journey, focused boundaries and unchanged existing consumers.
- [ ] 3.2 Run repository-required checks and strict OpenSpec validation on the integrated candidate.
- [ ] 3.3 Complete independent whole-change review and disposition findings.
- [ ] 3.4 Obtain human result acceptance; merge, release and caller/host activation remain separate decisions.

## Current state

Issue #34: proposal only. Its broad outcome and sequence are accepted; the detailed contract above is not yet accepted. No API, implementation, new test expectations, partial behavior or host effects are delivered by this change. The preceding CLI/LSP integrations do not complete #34.
