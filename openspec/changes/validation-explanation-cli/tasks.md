## 1. Scope and independent verification

- [x] 1.1 Record user scope acceptance and MR #26 baseline; verify issue decisions and merged commit `2f5ecbb`.
- [x] 1.2 Specify commands, context, exact output, explanation semantics, and failure boundaries; validate with OpenSpec.
- [x] 1.3 Independently author and freeze consumer-visible CLI checks; record their exact command, baseline gap, and ownership.

## 2. Implementation and integration

- [x] 2.1 Implement bounded core explanation through existing schema/resolver owners; verify accurate constraints and recursive semantics.
- [x] 2.2 Implement thin check/resolve/explain binary; verify the authored process checks and real file/stdin journeys.
- [x] 2.3 Confirm consolidated validation/representation ownership and simplify unjustified local structure; record examined path and disposition.

## 3. Verification and handoff

- [x] 3.1 Exercise real CLI native/recursive-module success and failure, provenance, exact values, and unchanged inputs; retain observed results.
- [x] 3.2 Reconcile user/API documentation, source-built availability, and release-owner integration boundary.
- [x] 3.3 Run repository-required checks and OpenSpec validation on the integrated revision; resolve failures without changing frozen checks.
- [x] 3.4 Obtain human result acceptance after independent whole-change review and finding disposition; release remains separately authorized.

## Delivery state

Issue #32's CLI result `c31daace273fe48e8b3e94f55e91172b20ae344f` is human-accepted. The user's later “bring it in” authorizes integration through [PR #35](https://github.com/dprb-work/daml/pull/35#issuecomment-6735508216), not publication or deployment. Issue #33 is a separate result; #34 remains unimplemented. Release/archive inclusion remains separately owned. The primary checkout is unchanged.

Independent check owners: `CliChecks` and `ExplanationChecks`; implementers do not edit their tests/fixtures. Frozen commands: `cargo test --locked --test configuration_cli` (test SHA-256 `3a31861548cd1d4da2248922494446ce70faee429b0a8b48fc257fa3146e834f`) and `cargo test --locked --test explanation` (test SHA-256 `2ec996d3075f27bec58e121a318e8bf935440eb5dc55fd1859edb86e39659bb3`). Check-owner amendments corrected invalid source/profile/API expectations and added branch/recursive coverage; expected consumer outcomes were preserved. The first CLI run passed seven cases and exposed three comma-separated source-fixture errors; their owner corrected the fixtures, after which all ten passed. Four core explanation checks passed. Prompt ownership is not hostile-write isolation. The baseline had no `daml` binary; no failing-before behavioral claim is made.

Coherent CLI slice challenge: `CliChallenge` inspected the binary and supporting contracts without running commands. No present CLI defect was established. Core explanation, assembled verification, and final independent review were retained as separate integration obligations rather than inferred from that slice.

Integrated smoke: native check/resolve/explain succeeds with actual defaults and input/default origins. Explicit application modules pass the documented check command. A recursive stdin document explains `/next/value` as number `"2"` with input origin and its minimum constraint; a negative recursive descendant exits 1 with empty stdout and `/next/value` plus schema location in stderr.

Consolidation examination: argument/context admission reaches the existing compiler/registry/resolver; explanation consumes the pinned validator's authoritative evaluation and shares MR #26 work admission. No second validator or recursive graph walker was added. Existing embedding examples and formatter I/O remain intentionally separate consumers with different teaching/output contracts; no obsolete owner or compatibility shim was found. Normal resolution allocates no explanation trace.

Simplification examination: borrowed Serde output streams exact checked data into one bounded result buffer without an intermediate JSON/semantic tree; only the existing DAML writer needs its owned semantic snapshot. The private zero-sized evaluation-path mode shares one cost walker without enlarging ordinary-validation tasks. Retain those concrete representations for their current output/budget contracts. Replaced the CLI help's escaped continuation literal after the actual run showed stripped indentation; its static bytes are borrowed through `Cow` rather than copied into a result allocation. No broader rewrite warranted.

Documentation: command reference and README/index/component links now distinguish source-built CLI availability from existing archives. Schema API, conversion boundaries, getting-started routing and architecture owners are reconciled. Grammar, binding APIs, release producers, and older release instructions are intentionally unchanged.

Final verification: `./scripts/check` passed after the help-ownership refinement, including Rust checks/tests/doctests/rustdoc and a rebuilt Python extension with 151 passing Python tests. Actual help and native check passed in the same invocation. Browser binding `cargo check --locked --manifest-path bindings/browser/Cargo.toml --target wasm32-unknown-unknown` passed on the unchanged core. Strict OpenSpec validation passed. Browser JavaScript/runtime verification was not run; no browser explanation API or UI change is claimed.

Additional actual CLI probes passed: conflicting-cwd bundle discovery, explicit stdin context, both sides of asserted/expected contract conjunction, private assertion refusal without implicit JSON fallback, a valid document whose JSON output hits the 8 MiB cap with empty stdout, and a closed output pipe returning 2. Conditional else constraints apply to the selected value while rejected then and property-name constraints do not.

Independent review: the whole-change reviewer inspected the staged diff and found no retained findings; security/resource review found no concrete defect, with static-allocation-analysis and broad-advisory-audit limits stated. The initial packet's exact-diff limitation was covered by the whole-change reviewer's own comparison. Final artifact identity and result/release decisions belong to the pull request/issue, not an inferred author approval.
