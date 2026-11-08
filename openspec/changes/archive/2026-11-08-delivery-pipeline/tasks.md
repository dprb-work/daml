## Scope

- [x] Reconcile the landed browser 0.2.0 baseline and define coordinated remaining shipping versions.
- [x] Define immutable candidate, qualification ownership and protected publication boundaries; no upload/tag/release authority.

## Implementation

- [x] Align remaining manifests/locks and remove version-pinned VSIX producer/runner paths.
- [x] Add provenance-bearing VSIX and real browser archive qualification to the existing graph.
- [x] Implement exact eight-payload candidate admission and collection with an approval digest.
- [x] Implement separately dispatched, protected artifact-only promotion with explicit partial-effect handling.
- [x] Configure the GitHub approval gate and document external PyPI onboarding/operator actions.

## Verification and acceptance

- [x] Exercise the real complete preparation preview and candidate corruption checks; verify no public effects.
- [x] Run required repository/OpenSpec checks and independently review the exact candidate.
- [x] Present the qualified result for human acceptance, separately from merge and publication authorization.

## Qualification record

The first complete preview passed at source
`f1bbc14a988099a3fca558a846dbdbfd61b0d593` in
[run 42522289143](https://github.com/dprb-work/daml/actions/runs/42522289143).
It exercised native archives and Python wheels on all three platforms,
CPython 3.12/3.13 consumers, installed VSIX/Electron, packed SDK types/native
parity/Chromium, and complete collection. The downloaded eight-payload candidate
also passed the independent corruption probe; actual Windows legal bytes and
the GitHub artifact archive digest were verified.

Independent artifact and security reviews have no remaining findings.
Repository checks, eighteen delivery-policy checks, twelve protected planner
checks, wheel admission checks and strict OpenSpec validation passed.
Final-head evidence and the pending human result decision are retained in
[PR 36](https://github.com/dprb-work/daml/pull/36).

No merge, tag, release, registry upload or PyPI publisher activation was
performed. Feature-branch preview artifacts are not eligible for publication.
Live publication-token/OIDC effects remain unexecuted; external Trusted
Publisher setup and separately authorized main preparation/publication are
still required. Completing this checklist does not accept or release the result.
