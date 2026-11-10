# Validation work — 2026-11-10

The Olaf Recipe blocker was **Cartesian completion search**, compounded by
conservative validation reservations. It was not a missing binding or an
insufficiently large justified work limit. This investigation starts from DAML
`f30e3240cc4cd6860ee028b4df29bd18b83fbe33`; the consuming integration used the
released `0.2.0` API at `710fd87b8e977fbd0dc706d74c180078254bd4d8`.

## Cause and correction

`Resolver::next_union` selected every default-bearing alternative before checking
whether it could intersect the supplied tag's schema. Independent nested unions
then formed a Cartesian product in `CompletionSearch`. Candidate validation
repeatedly rejected an early function while unrelated later choices advanced.

For the exact Recipe, the first candidate had 22 choices: fifteen binary choices
and seven 34-way choices. Their fixed-shape product is 1,721,085,137,518,592;
this is a search-shape calculation, **not an executed-candidate count**. A bounded
diagnostic reached candidate 100 with the same `/steps/0/fns/0` rejection before
exhausting a 100-million-unit allowance. The ordinary invocation stopped during
`check_node`'s diagnostic reservation at `/steps/0`.

Completion now rejects branches proved impossible by finite `const`/`enum`
intersections, including constraints on mandatory fields before defaults exist.
Ordinary resolution can also use supplied scalar values. Staged resolution uses
retained assertions and contracts, not mutable values. Tags contribute their
bound schema constraints; no tag-name membership rule or synthetic data field
is introduced. The original unions still validate completed values, preserving
`oneOf` overlap rejection and ambiguous-completion errors.

Validation-only unions bypass compatibility proofs. Finite intersections start
from the smallest available domain. Property-name hashing, required-name scans
and ordered property lookups reserve text work before running. Immutable schema
metadata stores the name-work bounds so estimating a lookup does not rescan its
names. Proofs visit fields lazily and stop at the first contradiction.

`SchemaView::evaluation_work` now owns both acyclic and recursive runtime
reservations. Unused definitions and default payloads no longer multiply every
instance check. `check_node` reserves a validity probe first, then separately
reserves eager diagnostics on failure. Both passes charge the same cumulative
budget before entering the engine. No global cache, second validator, dependency,
public API, or limit was added.

## Scaling measurements

`tests/validation_scaling.rs` provides a consumer-independent recursive catalog:
each alternative requires a distinct `kind`, supplies its default, and accepts
children through the same union. The assertion name deliberately differs from
the discriminator value. Root-only input supplies the equivalent `kind` data;
tagged input obtains it from the selected schema's default.

The table reports the minimum successful cumulative `max_steps`, found by
bounded binary search. `>1m` means the default allowance was exhausted, not a
timeout. These are conservative budget thresholds, not timings or engine visits.

| Depth | Alternatives | Root-only before | Root-only after | Tagged before | Tagged after |
|---:|---:|---:|---:|---:|---:|
| 1 | 4 | 4,672 | 1,211 | 8,900 | 1,467 |
| 2 | 4 | 123,762 | 3,808 | 410,541 | 4,673 |
| 3 | 4 | >1m | 12,053 | >1m | 14,939 |
| 4 | 4 | >1m | 42,890 | >1m | 53,445 |
| 3 | 1 | 3,905 | 1,526 | 5,857 | 2,210 |
| 3 | 2 | 71,360 | 3,498 | 285,986 | 4,748 |
| 3 | 8 | >1m | 61,146 | >1m | 69,254 |
| 3 | 16 | >1m | 398,141 | >1m | 424,595 |

Reproduce the diagnostic with:

```sh
cargo test --locked --test validation_scaling \
  report_validation_work_scaling_for_depth_alternatives_and_tags \
  -- --ignored --nocapture
```

The diagnostic also compares explicitly tagged input with a supplied discriminator.
Pruning removes impossible completion combinations, not every schema traversal.
Conservative reservations for recursive unions can still grow multiplicatively;
this change does not promise linear validation for arbitrary schemas.

## Exact Recipe and binding evidence

The unchanged `visual_builder_interactions.daml` source has SHA-256
`7e24049ac6da9167e6b6f5909aff0be8d9b3b4c516d26e079eb3b5f0780dda13`.
A task-local extractor captured the module envelope passed by Olaf's actual
`_native_schemas` owner to `compose_schema_modules`, without mirroring its
normalization. It contains 282 bindings, 158 resources, 6,159 JSON value nodes,
and 1,168 schema-position nodes. The authored semantic input has 47 nodes and
20 tagged paths. These counts describe the selected normalized envelope, not
the complete Olaf catalog.

Before the fix, native semantic resolution failed specifically with
`schema validation work exceeds step limit`, including with a diagnostic
100-million-unit allowance. Afterwards it resolves with the unchanged default
allowance; its minimum successful semantic work budget is 485,473 units. Only
the two-way optional `optimize` completion remains a search choice.

The rebuilt native Python extension resolves the same captured semantic tree.
Olaf's unchanged public `decode` returns `Recipe visual_builder_interactions`;
its staged `validate_source` path also succeeds. Python bytecode writes were
disabled and the new ABI3 extension was selected through `PYTHONPATH`, without
changing Olaf's checkout or installed dependency.

A development WASM build using the pinned `wasm-bindgen` 0.2.129 was exercised
in real Chromium. `Schemas.resolve` accepts the same source, retains its root
assertion/binding, and produces the same completed JSON as native resolution.
`validateSemantic` accepts that completed tree and rejects a numeric `setup_id`
with `SchemaViolation` at `/setup_id`. This is SDK evidence, not a rerun of
Olaf's rendered graph-authoring E2E suite.

## Protection and refinement

The 1-million-unit default work budget and 10,000-node aggregate schema bound
are unchanged. Focused regressions retain duplicate-recursive-work rejection,
exact numeric equality, optional-property omission, supplied-null precedence,
`oneOf` ambiguity, staged mutation and retained-tag obligations. Two individually
admitted modules exceeding the aggregate schema-node bound are still rejected.

Review regressions also cover large enum intersections and mandatory common
defaults under validation-only unions. Both initially failed with work exhaustion
despite inexpensive valid completions. A private proof-boundary test uses
100 schema names of about 16 KiB each and a budget smaller than one text pass;
it rejects before uncharged hashing or required-name comparisons, with and
without required fields. The rejected implementation returned `true` at that
boundary. Default-bearing optional/numeric cases exercise pruning rather than
merely checking the original validator's final rejection.

One existing reference/default uniqueness fixture depended on obsolete
whole-schema overreservation: 500 distinct integers fit its 10-million-unit
allowance after inert payload accounting was removed. The fixture now uses
2,000 integers at the **same allowance**, reserving 1,999,000 pairwise
comparisons. Ordinary validation still fits; referenced uniqueness remains
over budget. No production bound was weakened to satisfy that check.

The consolidation pass removed the separate acyclic runtime cost coefficient;
compilation admission retains its distinct graph accounting. The simplification
pass removed diagnostic scaffolding and an unnecessary context clone. Branch
proofs reuse schema conjunction, finite-domain comparison and the existing
cumulative budget rather than adding a parallel validation path.

All work remained local. No release package, publication, deployment, hosted CI,
Olaf edit, or separate browser-SDK worktree change is part of this repair.
