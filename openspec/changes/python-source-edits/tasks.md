# Tasks

## 1. Scope and checks
- [x] 1.1 Record owner approval of bounded one-shot Python editing and define the API against existing binding conventions.
- [x] 1.2 Independently author checks, rebuild the exact baseline, observe an intended missing-method failure and freeze the protected input set.

## 2. Implementation and integration
- [x] 2.1 Add bounded strict operation admission and one-shot dispatch through the existing Rust editor.
- [x] 2.2 Add public exception translation, method integration, exports and typing without a persistent borrowed patch object.
- [x] 2.3 Reconcile Python usage/reference and availability documentation; keep canonical writing distinct.
- [x] 2.4 Challenge the composed Python→Rust→new-document path and examine consolidation/simplification at the owning boundaries.

## 3. Verification and acceptance
- [x] 3.1 Exercise rebuilt-binding acceptance checks and real Python-derived schema workflows, including source preservation and failure boundaries.
- [x] 3.2 Run repository aggregate checks and strict OpenSpec validation on the integrated candidate.
- [x] 3.3 Complete independent exact-head whole-change review and disposition findings.
- [ ] 3.4 Obtain human result acceptance; merge, publication, installation and activation remain separate decisions.

## Current state

Issue #54 / PR #55: implementation, documentation and agent-owned verification are complete. The rebuilt binding passes all 139 independently owned acceptance cases and a real dataclass-derived schema workflow. `./scripts/check` passes, including all 290 Python tests; strict OpenSpec validation passes. Independent integrated challenge and fresh whole-change review found no actionable defects. The full review covers code head `67a5f343a8c5ec07eb456cf35d1361725f1c3f53`; this subsequent task-status update does not change code, tests, configuration or user documentation. Existing core editing, semantic admission and error translation remain canonical; borrowed expected strings avoid a new allocation. The bounded two-stage field check retains the existing stage-adapter shape/error convention. PR #55 remains draft awaiting human result acceptance. Hosted Required CI deliberately fails the draft tier and skips heavy verification; no hosted substantive pass, merge, publication, installation or activation is claimed.
