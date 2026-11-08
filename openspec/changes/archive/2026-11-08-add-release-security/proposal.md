# Proposal

## Why

The user selected one complete security suite as a required stage of deliberately
initiated release pipelines, with DAML's coordinated 0.2.0 release as the pilot.
Ordinary optimized CI must not start scanners on every merge or PR.

## What Changes

- Run all configured source, dependency, secret and artifact security checks during
  explicit release preparation, before the immutable approval packet is sealed.
- Embed matching evidence in RELEASE.json schema 2 without changing the eight
  payloads or introducing another artifact into the collector's input set.
- Require that evidence in common candidate admission, reused by publisher
  preflight and post-human-approval revalidation.
- Pin real scanner engines/configuration and make failure, coverage, freshness,
  source/artifact identity and network behavior explicit.

## Capabilities

### New Capabilities

- `release-security`: complete scanner execution and matching evidence admission.

### Modified Capabilities

- None. The new capability extends the candidate admission boundary while
  preserving the canonical [delivery-pipeline specification](../../specs/delivery-pipeline/spec.md),
  payload layout, publisher ownership and publication destinations.

## Impact

The pilot was authored in `/home/jason/wt/daml/feat/release-security`, based on
32ee55dcfe4447d937e6327d4c1bec6288b0c2bb. Release-side integration now belongs to
`/home/jason/wt/daml/build/release-pipeline` on `feat/release-security-integration`,
based on main a9a2e0993483f13c2134cd3a4482a6a82bd5ada0 after PR #36 merged.
The existing publisher remains responsible for environment approval, immutable
preparation origin, GitHub/PyPI effects and recovery. Imported verification inputs
need independent ownership amendment before assembled qualification. No release
is published, provider configured or budget changed. Template adoption remains
separately owned; no other consumers are migrated here.

## Scope decision

The user approved integration and selected “DAML pilot, then template,” naming
chat 01a1bbb9-c9da-751c-97c1-eb816e295bc5 for coordination. The release owner
accepted release-side reconciliation and qualification in the shared
`/home/jason/wt/daml/feat/release-security/openspec/changes/add-release-security/sync.md`.
Integration does not establish final acceptance or authorize bypassing GitHub's
human environment approval or PyPI Trusted Publisher setup.
