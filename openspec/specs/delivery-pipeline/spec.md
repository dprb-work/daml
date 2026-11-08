# delivery-pipeline Specification

## Purpose
Prepare a coherent, real-qualified DAML release candidate and promote only its owner-approved immutable files, with no automatic publication from source changes.

## Requirements

### Requirement: Coordinated shipping version

Core, Python, browser Cargo/npm and VS Code manifests SHALL declare the same canonical release version. The initial target SHALL be 0.2.0, preserving the already-landed browser version. Fuzz and isolated verifier package versions SHALL remain independent. A mismatch SHALL fail qualification before expensive build jobs.

#### Scenario: Drifted component

- **WHEN** one shipping manifest differs from the other release components
- **THEN** preparation fails without constructing a releasable candidate

### Requirement: Reuse the qualification graph

Explicit preparation SHALL run existing core, dependency, fuzz, native archive, Python wheel and installed-editor qualification at one immutable SHA, plus browser native parity, generated-type and real-browser checks. Regular selective CI, draft readiness and current-head/base guards SHALL remain intact.

#### Scenario: Premerge preview

- **WHEN** the existing Check workflow is manually dispatched at a delivery feature ref
- **THEN** it can prepare and exercise a complete candidate through the same shared graph
- **AND** that preview is not eligible for public promotion

### Requirement: Exact release bundle

A candidate SHALL contain exactly three supported native archives, three stable-ABI Python wheels, one browser SDK archive and one VSIX, plus eight matching SHA-256 sidecars and RELEASE.json. Its immutable source SHA, clean-source provenance, coordinated version and artifact-specific identities SHALL agree. Existing producer payloads SHALL be retained byte-for-byte; collection SHALL NOT rebuild or repack them.

#### Scenario: Generated or stale payload

- **WHEN** a payload is missing, extra, corrupt, dirty, from another revision or an inconsistent version
- **THEN** candidate admission fails before promotion

#### Scenario: Approval identity

- **WHEN** a complete candidate is prepared
- **THEN** the operator receives its version, source SHA, preparation run ID, manifest SHA-256 and file identities
- **AND** a new or changed manifest requires a new approval decision

### Requirement: Protected artifact-only publication

Publication SHALL require an explicit dispatch bound to version, source SHA, canonical preparation run ID and manifest digest. The preparation SHALL be successful, originate from the same repository's canonical prepare-release.yml on main, and identify the current main source SHA. The publisher SHALL re-admit this state after human approval before effects. Feature previews, moved main tips and mismatched workflow/source identities SHALL be rejected.

#### Scenario: Stale approval

- **WHEN** main moves after preparation or while the publication job awaits approval
- **THEN** publication stops before external writes and requires a fresh candidate/decision

### Requirement: Fail-closed human environment

Read-only preflight SHALL verify that the release environment already exists with the named human reviewer, prevention of self-review and a main-only deployment policy. It SHALL NOT reference an absent environment in a way that auto-creates an unprotected publication job. Write credentials and OIDC permission SHALL be limited to the protected publication job.

#### Scenario: Missing protection

- **WHEN** the configured environment or required reviewer/branch protections are missing or weakened
- **THEN** preflight fails and no privileged publication job starts

### Requirement: Supported provider operations and honest effects

The publisher SHALL use the official pinned PyPA Trusted Publishing action, not stored PyPI tokens or a custom OIDC client. After approval it SHALL stage a GitHub draft/tag/assets first, upload only the three unchanged wheels to PyPI, observe the public registry hashes, and make only that newly staged GitHub release public. It SHALL NOT overwrite existing tags/assets/package versions, silently rebuild files, use blanket skip-existing, or automatically retry/recover partial writes.

#### Scenario: Partial external completion

- **WHEN** a provider fails after a draft/tag or registry files have been created
- **THEN** the workflow reports those possible effects and requires owner inspection and fresh recovery authorization
- **AND** it does not claim atomic rollback or delete published state automatically

### Requirement: Preparation does not grant publication authority

This delivery change SHALL be verified through real non-publishing preparation, artifact admission and installed consumers. Publication credentials, tags, release creation and registry uploads SHALL NOT be exercised without a later explicit owner authorization. Operator guidance SHALL identify the PyPI Trusted Publisher prerequisite and distinguish verified preparation from unexecuted publication effects.

#### Scenario: Delivery-pipeline handoff

- **WHEN** implementation and preparation verification are complete
- **THEN** the handoff names the exact qualified revision, evidence and outstanding owner-side publishing setup
- **AND** it does not imply that DAML has been uploaded or publicly released
