## Preparation

- [x] Inspect current package/release owners and PyPI namespace; record the user's `daml-config` choice.
- [x] Define native platform/stable-ABI boundaries and split producer, verification and integration ownership.

## Implementation

- [x] Implement distribution metadata migration and the canonical wheel producer with legal/provenance payloads.
- [x] Add independently owned artifact admission and isolated installed-consumer verification.
- [x] Connect wheel preparation and CPython 3.12/3.13 verification to the existing native runner matrix without publication effects.
- [x] Update source/candidate installation and developer packaging documentation without implying registry publication.

## Verification and handoff

- [ ] Build a clean candidate wheel locally and verify the installed base and types workflows outside the checkout.
- [ ] Run the required repository gate and strict OpenSpec validation; obtain exact-candidate native matrix evidence.
- [ ] Complete independent final review and disposition findings at the exact verified head.
- [ ] Present result acceptance separately from merge and artifact-publication authority.
