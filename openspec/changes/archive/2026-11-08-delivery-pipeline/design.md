## Existing owners

Cargo/npm manifests remain authoritative for component versions. The coordinated check compares five shipping declarations; lockfiles are regenerated through their existing tools. Browser 0.2.0 is the accepted main baseline, not work to recreate.

Existing native, Python, editor and browser producers own their formats. Collection adds one release identity/approval manifest, not a second binary builder. The qualification graph reuses ci-heavy.yml; explicit preparation adds browser qualification and collection while regular selective checks retain the existing planner contract. The established Check dispatch provides a real feature-branch preview before a newly introduced canonical workflow is registered on main.

Maturin remains the wheel/RECORD owner; npm owns SDK packing; VSCE owns VSIX packing. The VSIX producer gains a real current responsibility: version-derived output naming, canonical source/legal payloads, build provenance and checksums. Installed-host verifiers take the exact built VSIX path instead of another hardcoded version.

## Approval and provider boundary

Preparation has read-only permissions and produces eight payloads, eight checksums and a deterministic RELEASE.json. An operator approves the exact version/source/run/digest tuple. Promotion downloads and re-admits that exact snapshot after the environment approval; no mutable workspace handoff or floating latest artifact is trusted.

The protected `release` environment uses the existing human approver `dprbook`, prevents self-review and restricts deployment to main. Preflight reads the environment before the dependent job references it, avoiding GitHub's unprotected auto-creation behavior. No stored package-registry token is introduced. The official PyPA action receives OIDC permission only inside the approved job. The environment and pending/normal PyPI Trusted Publisher are separate configuration responsibilities; this task does not activate the latter or upload a package.

## Current-main restriction

Older-main promotion was considered, then rejected before implementation acceptance. GitHub documents additional workflow-write scope for release targets whose workflow files differ from the default branch. The pipeline does not acquire a broader PAT or try another API to work around that restriction. Qualification source, publication dispatch SHA and current main must agree; main moving during the approval window requires a new candidate. This trades release-window flexibility for a smaller permission and source-admission contract.

## Publication ordering and recovery

The approved publisher explicitly creates and verifies the public Git tag, then stages the draft and assets before PyPI. A draft alone does not establish that tag creation is permitted. Current-main admission still precedes the first write; no API is used to bypass workflow-write restrictions. Public registry file hashes are checked before the draft becomes public. These providers do not offer a shared transaction: a failed attempt may leave a public tag, draft, assets or some/all registry files.

Existing objects are not overwritten. There is no automatic retry, rollback or skip-existing mode. Operators inspect the original run, exact hashes and public state, then obtain a new bounded recovery decision. A read-only plan describes state without pretending that unexecuted publication has been verified.

## Verification ownership

An independent owner authors version/origin/environment policy checks and a corruption probe that first requires a real complete bundle. No synthetic compiled artifact is used as runtime proof. It also owns changes to existing installed-VSIX runner inputs. Protected input identities and execution notes live in ignored target/.agent state, not maintained documentation. This is procedural coordination, not hostile write isolation.

Main exercises real preparation, archive/wheel/editor/browser consumers and repository gates, then freezes an exact review head for independent review. Publication remains unexecuted because exposure authority and PyPI onboarding are separate prerequisites.
