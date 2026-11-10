# Prepare and publish a DAML release

A release is a qualified set of files, not a build performed during upload.
The preparation workflow builds and exercises those files. A separate workflow
promotes only the version, source revision, run and manifest digest that the
owner approves, to the separately approved publication target.

The coordinated source version is **0.2.1**. It is a release candidate until
publication completes. Existing 0.2.0 releases remain available and unchanged.

The owner selected **GitHub-only publication for v0.2.1**. PyPI publication and
account setup are deferred. The three qualified wheels still ship as GitHub
release attachments for manual download and installation; this does not make
`daml-config` available from PyPI.

## What ships together

| Component | Payloads | Destination after approval |
|---|---|---|
| Formatter/LSP | Linux x64, macOS ARM64 and Windows x64 native archives | GitHub release `v0.2.1` |
| Python | Three `daml-config` stable-ABI wheels; import remains `daml` | The same GitHub release; also PyPI only in `github-and-pypi` mode |
| Browser SDK | `dprb-work-daml-browser-0.2.1.tgz` | The same GitHub release; not npm |
| VS Code | `definitively-asserted-daml-0.2.1.vsix` | The same GitHub release; not a marketplace |

The candidate contains eight payloads, eight SHA-256 sidecars and `RELEASE.json`.
It contains no Python sdist or published Cargo crate. The existing
[distribution guide](distribution.md) describes platform requirements and legal
payloads. Checksums detect corruption; the repository, qualified run and approved
manifest establish which files the operator intends to publish.

## Choose the publication target explicitly

Every publication tuple command (`plan`, `preflight`, `stage-github`, `promote`)
requires `--publication-target`. The workflow requires the matching
`publication_target` input. There is no implicit target or fallback.

- `github-only` creates the tag and GitHub release with all qualified files.
  It makes no PyPI requests, prepares no registry wheel directory and performs
  no registry upload. Release notes identify the attached wheels as downloads
  for installation, not as a PyPI publication.
- `github-and-pypi` also uploads the three unchanged wheels through the official
  PyPA action. It requires the PyPI version to be absent before staging and
  observes the exact public wheel hashes before publishing the GitHub draft.

Both modes require the same qualified candidate, source/run/manifest binding,
current-main checks and protected human approval. The target describes publication
effects; it does not change `RELEASE.json` schema 2 or any producer output.
Changing the target requires an explicit owner decision, not an automatic retry.

## Keep versions coherent

The root Cargo manifest, Python Cargo manifest, browser Cargo/npm manifests and
VS Code manifest must agree. All shipping components are advanced together;
private fuzz/verifier package versions
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

## PyPI prerequisites for `github-and-pypi` only

Skip this section for `github-only`; no PyPI account, publisher configuration or
authentication setup is needed for that mode.

Before an explicitly authorized `github-and-pypi` upload, the approved PyPI
account owner must configure a pending publisher for the new project, or a normal
publisher if it already exists. Source preparation and GitHub-only publication
do not activate that external authority. Use the
[official Trusted Publishing guidance](https://docs.pypi.org/trusted-publishers/using-a-publisher/).

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

### Full security suite before candidate retention

Release preparation runs every configured security check after admitting the
unchanged producer payloads and before sealing the approval manifest. There is
no separate scanner-selection button. Ordinary PR, merge, push and branch-promotion
verification does not invoke this suite.

The configured engines are Trivy for dependency and current-source secret checks,
Semgrep CE with local source rules, and ClamAV for the delivered payloads. Engine
images are pinned by digest. Preparation refreshes the required advisory/signature
data without mounting source; scans run offline against read-only snapshots.
No credentials, platform login, source submission or malware-sample upload is used.
See `security/README.md` for exact versions, coverage and resource requirements.

The suite records per-check findings and errors. Any failed, missing, stale or
incomplete required check prevents a publishable candidate. Independent checks
can still report after another fails; a partial suite is never a pass.

`RELEASE.json` schema 2 embeds the evidence under `security`, binding the source,
exact payload records, policy/rules, tools and database identities. The approval
manifest digest covers that evidence without adding another candidate file or
mixing scanner report artifacts with the eight producer directories. Schema-1
candidates are not eligible: prepare a new schema-2 candidate and approve its new
manifest digest.

The publisher's existing bundle admission checks this evidence both before and
after human approval. A green job or successful scan alone does not authorize
publication. If content changes or the evidence expires, prepare and approve a new
candidate rather than bypassing the guard.

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
  --version 0.2.1 --source-sha SOURCE_SHA --prepare-run-id RUN_ID \
  --manifest-sha256 MANIFEST_SHA256
```

Use a new output directory; do not mix runs. Actions artifacts expire after 14
days, but security evidence and required database freshness expire within 24
hours under the current policy, including time spent awaiting human approval.
If either expires or the candidate identity changes, prepare and approve a new
candidate. Do not silently rebuild files or refresh evidence in the publisher.

## Authorize publication

The owner must approve the exact version/source/run/manifest tuple **and** the
publication target. For `github-only`, approval authorizes the GitHub tag, draft
and public release, not PyPI effects. `github-and-pypi` additionally authorizes
the registry upload. Successful preparation, a merged pull request or a green
check alone does not supply that authorization.

Publication requires that the qualified source still be the current `main`
tip and the dispatch SHA. If main advances during preparation or approval,
prepare a new candidate. This avoids broader workflow-write credentials for
stale targets; there is no alternate-API workaround or privileged PAT.

After that explicit decision, the authorized automation account dispatches:

```sh
gh workflow run publish-release.yml --repo dprb-work/daml --ref main \
  -f version=0.2.1 -f source_sha=SOURCE_SHA \
  -f prepare_run_id=RUN_ID -f manifest_sha256=MANIFEST_SHA256 \
  -f publication_target=github-only
```

The example uses the owner-selected v0.2.1 target. Use `github-and-pypi` only
after a separate explicit decision authorizes that target and its prerequisites.
The run name and approval summary identify the selected target alongside the
immutable candidate tuple. Write commands require that their explicit target
match the workflow's `RELEASE_TARGET`; a mismatch fails before external writes.

The read-only preflight checks the canonical successful main preparation,
protected environment and exact candidate bytes. The protected job downloads
and checks the same immutable artifact again after human approval. `dprbook`
approves through GitHub's environment review in **both modes**; the triggering
account cannot self-approve. GitHub-only mode does not bypass this review.

The approved job then:

1. Creates and verifies the public Git tag at the approved source, then stages
   a GitHub draft with all approved files, without overwrite.
2. In `github-and-pypi` mode only, uploads the three unchanged wheels through the
   official PyPA action and observes the public PyPI version's exact wheel hashes.
3. Makes only that newly staged GitHub draft public. In `github-only` mode, the
   wheels are attached for download and installation, not published to PyPI.

No source build, blanket `skip-existing` or automatic write retry occurs in that job.
After one successful tag-creation response, verification permits six exact-ref
reads with delays of 0, 1, 2, 4, 8 and 15 seconds. Only an absent ref (HTTP 404)
is retried; a wrong target or other API error stops immediately. Existing tags
remain an admission failure. Main is checked again before any draft write.
Persistent absence reports the already-created tag as a partial effect requiring
read-only inspection and an authorized recovery decision.
The tag-first order exercises GitHub tag permissions/rulesets before irreversible
registry upload; a draft alone does not prove that permission. A failure can leave
the tag publicly visible even when the release remains a draft. Rust crates, npm
and editor marketplaces remain untouched.

## Inspect partial effects before recovery

GitHub and PyPI do not share a transaction. A failed `github-only` attempt may
leave a tag, draft or assets. A failed `github-and-pypi` attempt may also leave
some or all PyPI files. A failed run does not mean nothing happened.

Use the read-only plan route with the original approved tuple and target:

```sh
python3 scripts/release_publish.py plan \
  --version 0.2.1 --source-sha SOURCE_SHA --prepare-run-id RUN_ID \
  --manifest-sha256 MANIFEST_SHA256 --publication-target github-only
```

Inspect the original run, GitHub tag/draft and exact hashes. For
`github-and-pypi`, also inspect public registry metadata; a `github-only` plan
does not query PyPI. Keep the original target when inspecting a partial attempt.
If main has moved, retain that fact in the recovery decision; do not
bypass the normal current-main admission. Obtain fresh owner authorization for
only the remaining effects. Do not rerun blindly, delete published state,
replace a version, force-upload assets or regenerate different bytes under the
same version.

After a completed `github-only` release, verify installation from the attached
wheel outside the checkout. Download the wheel for the supported platform,
check its SHA-256 sidecar and install that file with
`python -m pip install /path/to/daml_config-0.2.1-cp312-abi3-manylinux_2_35_x86_64.whl`
for the Linux x64 attachment; use the matching wheel on macOS ARM64 or Windows
x64. Do not claim a registry installation is available.

After a completed `github-and-pypi` release, verify a fresh registry installation
outside the checkout. Update user-facing availability and installation guidance
from observed published state. This guide describes the procedure; it does not
assert that publication or recovery has completed.

## v0.2.1 recovery record

[v0.2.1](https://github.com/dprb-work/daml/releases/tag/v0.2.1) was published
GitHub-only from `9f6dcb0eb5d530b56737dd3bad23fcdee22030b0`, using preparation
[42785680584](https://github.com/dprb-work/daml/actions/runs/42785680584),
artifact `13750395007`, and manifest SHA-256
`f1f0c74ed5507f1e06845f6db1fd20742834af15970bf9b2ecb035c23905e7ce`.
The original protected run
[42879188270](https://github.com/dprb-work/daml/actions/runs/42879188270),
approved by `dprbook`, created the correct tag but stopped on an immediate
verification-read 404. No draft existed at that point.

The owner separately authorized remaining-effects recovery and then explicitly
authorized a one-time current-main exception after documentation-only PR #51
advanced main to `dbe083581c2564cf4e2752c8b1c02b970b67b30e`. Recovery required
that exact main revision, the original source as its ancestor, the unchanged
tag and qualified candidate, unchanged environment protections, the original
review record, fresh security admission, and no competing publisher.
This was an explicit exception, not satisfaction or removal of normal
current-main admission.

Recovery created and verified draft `440812617`, then published only that
draft. All 17 public asset sizes and SHA-256 digests matched the approved files.
The public Linux wheel, checksum and manifest matched the original bytes;
installation outside the checkout passed nested tagged-default resolution and
supplied-null rejection. No tag moved, payload was rebuilt or overwritten, or
registry upload occurred.
