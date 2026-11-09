# release-security Specification

## Purpose
Require a complete configured security suite for the exact DAML release candidate
before the separately authorized publisher can expose it.

## Requirements

### Requirement: Full security suite during explicit preparation

Explicit canonical preparation and its deliberate qualification preview SHALL run
all configured required checks against the immutable source and admitted payloads.
Ordinary optimized PR/push/merge/promotion CI SHALL NOT invoke these scanners.

#### Scenario: Prepare coordinated release payloads

- **GIVEN** all eight unchanged producer payloads admitted for one source/run
- **WHEN** collection prepares a publishable approval packet
- **THEN** all configured dependency, secret, source and malware checks execute
- **AND** the final manifest embeds complete per-check security evidence.

### Requirement: Matching evidence is required at publication admission

Candidate admission SHALL reject missing, failed, incomplete, stale or malformed
security proof, including changed source, payload, policy or rule identities.
Security success SHALL NOT waive native qualification or release authority.

#### Scenario: Revalidate after human approval

- **GIVEN** approved version/source/run/manifest and immutable artifact identity
- **WHEN** the publisher redownloads the candidate after approval
- **THEN** common admission verifies both real payload bytes and matching security
  evidence before tag, draft, registry or public-release effects.

#### Scenario: A configured check did not pass

- **GIVEN** a required check with findings, missing tools/data or partial execution
- **WHEN** preparation or publication evaluates the aggregate
- **THEN** no publishable candidate is accepted
- **AND** independent check outcomes remain visible without treating errors as clean.

### Requirement: Exact candidate layout and portable scanner ownership

The eight-payload/eight-sidecar/RELEASE.json layout SHALL remain intact. Security
proof SHALL bind payload records without a circular final-manifest hash and use
repository-owned portable code/configuration with immutable engine pins.

#### Scenario: Seal the approval digest

- **GIVEN** completed scanner results for the admitted payload hashes
- **WHEN** RELEASE.json schema 2 is written
- **THEN** its approved digest covers the embedded security evidence
- **AND** no extra report directory collides with producer artifact collection.
