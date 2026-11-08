# Prepare and publish a DAML release

A release is a qualified set of files, not a build performed during upload.
The preparation workflow builds and exercises those files. A separate workflow
promotes only the version, source revision, run and manifest digest that the
owner approves.

The coordinated source version is **0.2.0**. That version is not a claim that
0.2.0 has been published. Existing 0.1.0 releases remain available and unchanged.

## What ships together

| Component | Payloads | Destination after approval |
|---|---|---|
| Formatter/LSP | Linux x64, macOS ARM64 and Windows x64 native archives | GitHub release `v0.2.0` |
| Python | Three `daml-config` stable-ABI wheels; import remains `daml` | PyPI and the same GitHub release |
| Browser SDK | `dprb-work-daml-browser-0.2.0.tgz` | The same GitHub release; not npm |
| VS Code | `definitively-asserted-daml-0.2.0.vsix` | The same GitHub release; not a marketplace |

The candidate contains eight payloads, eight SHA-256 sidecars and `RELEASE.json`.
It contains no Python sdist or published Cargo crate. The existing
[distribution guide](distribution.md) describes platform requirements and legal
payloads. Checksums detect corruption; the repository, qualified run and approved
manifest establish which files the operator intends to publish.

## Keep versions coherent

The root Cargo manifest, Python Cargo manifest, browser Cargo/npm manifests and
VS Code manifest must agree. The browser's landed 0.2.0 is preserved; the other
shipping components are aligned to it. Private fuzz/verifier package versions
are not release versions.

Run the cheap source check before building:

```sh
python3 scripts/release_common.py check-version
```

When preparing another version, update those shipping declarations and path
version constraints, then regenerate Cargo, npm and uv lockfiles with their
own tools. Do not replace historical versioned release URLs or fabricate a
second maintained version list. VSIX output names come from its producer;
installed-host tests receive that exact path through `DAML_VSIX_PATH`.

## GitHub approval gate

The repository's `release` environment is configured with:

- Human reviewer `dprbook`.
- Prevention of self-review.
- A deployment branch policy allowing only the branch `main`, not arbitrary
  tags or matching feature branches.

The publication preflight reads these protections before a dependent job
references the environment. A missing environment must fail closed rather than
be auto-created without reviewers. Check the current server configuration:

```sh
python3 scripts/release_publish.py check-environment
```

These are normal repository protections, not isolation against a repository
administrator who deliberately changes them. No claim is made about an
administrator-bypass setting not exposed by the documented REST configuration.
Do not weaken the required review to make a publication run proceed.

## Configure PyPI Trusted Publishing

Before the first upload, the approved PyPI account owner must configure a
pending publisher for the new project, or a normal publisher if it already
exists. This pipeline does not activate that external authority during source
preparation. Use the [official Trusted Publishing guidance](https://docs.pypi.org/trusted-publishers/using-a-publisher/).

| Setting | Required value |
|---|---|
| PyPI project | `daml-config` |
| GitHub owner | `dprb-work` |
| Repository | `daml` |
| Workflow filename | `publish-release.yml` |
| Environment | `release` |

The official pinned PyPA action obtains short-lived OIDC credentials. Do not add
`PYPI_TOKEN`, a password fallback or a homemade token exchange. Passwords,
verification codes and tokens do not belong in chat, source or logs. The PyPI
project named `daml` is an unrelated library; do not publish there.

## Preview preparation before merge

The registered `Check` workflow supports a real feature-branch preview through
its manual full-check dispatch:

```sh
gh workflow run check.yml --repo dprb-work/daml --ref YOUR_BRANCH
```

Manual dispatch enables release preparation in the existing shared graph.
Regular pull-request/main checks keep their selective planner and draft/current
head/base gates. The preview adds the browser build, native parity, generated
consumer typecheck and real Chromium execution of the packed SDK, then collects
all eight producer payloads. It runs the independent candidate corruption probe
before retaining the bundle.

A feature-branch `Check` preview is **not eligible for publication**. It proves
the delivery machinery without granting write permissions or creating a tag,
release or registry package.

## Prepare the actual main candidate

After source acceptance and merge, use the canonical workflow:

```sh
gh workflow run prepare-release.yml --repo dprb-work/daml --ref main
```

Wait for the identified run, rather than selecting an unrelated latest run:

```sh
gh run watch RUN_ID --repo dprb-work/daml --exit-status
gh run download RUN_ID --repo dprb-work/daml \
  --name daml-release-candidate --dir candidate
```

The run summary gives the coordinated version, full source SHA, preparation run
ID and SHA-256 of `RELEASE.json`. The manifest records every payload's filename,
size, digest, kind and target. Qualification includes the existing native archive,
Python 3.12/3.13, installed VSIX, core, dependency and fuzz checks, plus the
browser path. The collector does not repack a producer's payload.

Validate the downloaded candidate using the tuple from that run summary:

```sh
python3 -B tests/release_candidate_check.py candidate \
  --version 0.2.0 --source-sha SOURCE_SHA --prepare-run-id RUN_ID \
  --manifest-sha256 MANIFEST_SHA256
```

Use a new output directory; do not mix runs. Actions artifacts expire after 14
days. If the candidate expires or its identity changes, prepare and approve a
new candidate. Do not silently rebuild files in the publisher.

## Authorize publication

The owner must approve the exact version/source/run/manifest tuple and the
GitHub tag/release and PyPI effects. Successful preparation, a merged pull request
or a green check alone does not supply that authorization.

Publication requires that the qualified source still be the current `main`
tip and the dispatch SHA. If main advances during preparation or approval,
prepare a new candidate. This avoids broader workflow-write credentials for
stale targets; there is no alternate-API workaround or privileged PAT.

After that explicit decision, the authorized automation account dispatches:

```sh
gh workflow run publish-release.yml --repo dprb-work/daml --ref main \
  -f version=0.2.0 -f source_sha=SOURCE_SHA \
  -f prepare_run_id=RUN_ID -f manifest_sha256=MANIFEST_SHA256
```

The read-only preflight checks the canonical successful main preparation,
protected environment and exact candidate bytes. The protected job downloads
and checks the same immutable artifact again after human approval. `dprbook`
approves through GitHub's environment review; the triggering account cannot
self-approve.

The approved job then:

1. Creates and verifies the public Git tag at the approved source, then stages
   a GitHub draft with all approved files, without overwrite.
2. Uploads only the three unchanged wheels through the official PyPA action.
3. Observes the public PyPI version's exact wheel hashes.
4. Makes only that newly staged GitHub draft public.

No source build, blanket `skip-existing` or automatic retry occurs in that job.
The tag-first order exercises GitHub tag permissions/rulesets before irreversible
registry upload; a draft alone does not prove that permission. A failure can leave
the tag publicly visible even when the release remains a draft. Rust crates, npm
and editor marketplaces remain untouched.

## Inspect partial effects before recovery

GitHub and PyPI do not share a transaction. A failed attempt may leave a tag,
draft, assets, or some/all PyPI files. A failed run does not mean nothing happened.

Use the read-only plan route with the original approved tuple:

```sh
python3 scripts/release_publish.py plan \
  --version 0.2.0 --source-sha SOURCE_SHA --prepare-run-id RUN_ID \
  --manifest-sha256 MANIFEST_SHA256
```

Inspect the original run, public registry metadata, GitHub tag/draft and exact
hashes. If main has moved, retain that fact in the recovery decision; do not
bypass the normal current-main admission. Obtain fresh owner authorization for
only the remaining effects. Do not rerun blindly, delete published state,
replace a version, force-upload assets or regenerate different bytes under the
same version.

After a completed release, verify a fresh registry installation outside the
checkout and update the user-facing availability/installation guidance from
observed published state. Publication and recovery were not executed as part
of preparing this delivery pipeline.
