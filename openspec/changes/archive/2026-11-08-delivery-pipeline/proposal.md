## Why

DAML now has qualified native/Python/editor artifacts and a landed browser SDK 0.2.0, but shipping still depends on manual promotion. Native, Python and editor manifests still declare 0.1.0. A release needs one coherent version and an exact, independently qualified artifact set before any public upload.

## What Changes

- Preserve the landed browser 0.2.0 baseline from PR #26; align core, Python and VS Code to 0.2.0 and regenerate their owned lockfiles. No schema/API work is redone.
- Reuse the existing heavy qualification graph. Add a cheap coordinated-version check, browser qualification and exact release-bundle collection in explicit preparation mode. Regular narrow CI selection and draft/current-head/base protections remain intact.
- Prepare three native archives, three Python wheels, one browser SDK archive and one VSIX, their checksums and a deterministic RELEASE.json approval packet. Every payload comes from the same immutable clean source revision; payloads are not rebuilt or repacked during promotion.
- Add a separately dispatched publication workflow with read-only preflight, a protected human approval environment, official PyPI Trusted Publishing, and exact-file GitHub release promotion.
- Configure the GitHub approval gate without publishing anything; document PyPI publisher onboarding and operator recovery. No secrets or homemade OIDC exchange are introduced.

## Capabilities

### New Capabilities

- `delivery-pipeline`: coherent release preparation and separately authorized artifact-only publication.

### Modified Capabilities

None. Library semantics and the already-landed recursive types remain unchanged.

## Impact

Shipping manifests/locks, VSIX packaging/provenance, a browser qualification workflow, the shared qualification graph, release policy/collector/promotion scripts, independent verification inputs and operator documentation change. Existing 0.1.0 release assets remain immutable and available. Rust crates, npm and the editor marketplace are not published by this pipeline; Python sdists remain outside the qualified route.

## Scope and authority

The user accepted preparing the next 0.2.0 delivery pipeline and reported that browser 0.2.0 had landed. The implementation uses main at `2f5ecbb7047d1b16daa8eb328aacea233f1b3e41`, preserving that work and aligning only remaining component versions. Scope includes non-publishing qualification and protected GitHub-side preparation, not public release/tag creation, PyPI upload, merge, paid CI activation or budget changes.

Publication will require a fresh explicit owner decision bound to version, source SHA, successful canonical preparation run and manifest digest. The GitHub environment requires the existing human approver `dprbook`; the automation account cannot self-approve.
