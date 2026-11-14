# CI checks and path selection

DAML classifies changed paths before installing Rust, Node.js, fuzzing, or archive
build tools. `scripts/ci-plan.py` owns the repository-specific path map. It accepts
an explicit mode and a newline-separated file of repository-relative paths:

```sh
python3 scripts/ci-plan.py --mode ready --paths-file changed-paths.txt
```

The JSON result reports a tier and stable check identities: `docs`, `core`,
`dependencies`, `fuzz`, `editors`, and `binaries`.
The automatic `Check` workflow runs the protected planner contract once, calls
only selected reusable heavy groups, and emits one `Check / Required` aggregate
for all selected identities. For PRs, the plan admits only the event merge commit
whose parents match the current head and live target-branch tip. It passes that
immutable SHA to every heavy group. The aggregate rechecks the head, target ref,
tip, and draft state. Malformed, incomplete, stale, missing, failed, or
cancelled work cannot pass.

The `Check` workflow exposes its read-only GitHub token only to isolated PR/ref
lookup steps. Planner and aggregate code run without it. Pull-request paths come
from a complete `git diff --no-renames --name-only -z` result; invalid or
unsupported path bytes fail closed instead of truncating the change set.

## Shared local and hosted source checks

`scripts/ci-plan.py` is the only path-to-check-set owner.
`scripts/check-tier` runs a named source group; `scripts/check-tier full`
asks that planner for the complete source set. The tracked pre-push hook and
documented `scripts/check-changes` command use the ready-PR route, reject dirty
or changed outgoing heads, and run every selected group through that runner.
There is no bypass option or local receipt store.

The hosted workflows prepare pinned tools, route the selected groups onto
parallel clean runners, and report statuses/artifacts. Local execution is
sequential and uses the prepared developer environment. Hosted PR merge/base
freshness and draft/metadata admission remain hosted responsibilities.
Headless sandbox configuration is machine preparation, not a different test
set. Source checks need no service containers or secrets.

The runner owns the formerly inline version/admission checks, Cargo policy,
sanitizer targets, npm audits/typecheck/VSIX packaging, release LSP build,
grammar checks, and complete installed Electron aggregate. Core still delegates
to `scripts/check`. Release-only platform/wheel/browser/security qualification
keeps its existing separate exact-candidate graph and human publishing gate.
Local editor verification removes only the VSIX/checksum it successfully
produced, including after a later check fails. Pre-existing candidates still
cause the producer to refuse and are never removed. Hosted output stays
available for the following artifact-upload step.


## Tiers

- **Draft:** classification runs, but no automatic heavy workflow runs. Both
  `converted_to_draft` and `ready_for_review` refresh the aggregate; a draft
  result fails rather than treating skipped verification as a pass.
- **Docs:** only the lightweight `git diff --check` documentation check runs
  within the planner job.
- **Cheap:** known source/test/example/Python-binding changes run `core`.
  Integration and semantic-editor fixture changes also run `editors`.
- **Full source:** shared manifests/locks, toolchains, dependency policy, fuzz
  harnesses, CI/setup/control files, and unknown paths automatically select all
  source identities on the PR itself. No dispatch proof or admission lookup is
  needed. Invalid or incomplete path input fails planning.
- **Push:** default-branch pushes run only planner/admission and whitespace
  checks, never heavy source verification.
- **Manual full:** dispatching `Check` runs every check group through the same
  planner and additionally qualifies the packed browser SDK and collects a complete
  release candidate. This is a non-publishing preparation preview; it does not make
  a feature-branch candidate eligible for promotion. The standalone `Editor
  integrations` and `Binaries` workflows remain available for their verification
  contracts; only `Binaries` retains its prior manual-dispatch entrypoint.

### Nightly source verification

At 03:47 UTC, `Check` runs all remaining source groups on the default-branch
snapshot: core, dependencies, fuzz (including AddressSanitizer seeds), and the
real editor/LSP integration suite. No platform binaries, browser release SDK,
release candidate collection, or release-security scans are added to this route.
Full source runs nightly or by advisory dispatch; risky PRs also run it
automatically. Ordinary source PRs intentionally lose pre-merge sanitizer and
shared-engine installed-editor coverage. Nightly detects those failures after
merge; release qualification remains exact-candidate and fail-closed.

Before heavy setup, one Actions query reads the head of the last successful
scheduled `Check` run. An equal head skips the source work and reports
`unchanged`, not a new full-suite pass; missing history or lookup failure runs
the source suite. This simple comparison does not track intervening failures or
detect changed external advisories on unchanged source.

Dispatch `Check` with `mode=nightly` to run source verification without the
unchanged-head skip. It is advisory and does not qualify a release or admit PR
proof. The default dispatch remains the existing complete release preview.
`Nightly source` is separate from the PR's stable `Required` identity.
A separate no-checkout reporter opens/updates the single `CI nightly: DAML`
issue on failure and closes an open issue on recovery. Already-closed issues
receive no new green-night comments. Actions must permit issue writes;
the orchestrator must also inspect missing/cancelled runs or reporter failures.


An `edited` PR event is metadata-only only when `changes` is nonempty, contains
only `title` or `body`, and each value has exactly a `from` field. A previous
body may be null. That run skips every later planner step and the heavy wrapper;
the aggregate is skipped under a distinct non-required name expression, so it
cannot replace the stable `Check / Required` status. GitHub may display the
unevaluated expression rather than the intended metadata label. Title/body
edits use a separate concurrency
group and do not cancel verification runs. Base edits, mixed edits, and
malformed changes take the normal full classification path.

## Positive documentation map

`scripts/ci-plan.py` owns the positive prose map: selected root/integration
README files, contributor guidance, Markdown under `docs/`, and authored
OpenSpec Markdown/`.openspec.yaml` metadata. Runtime skill Markdown, executable
YAML, and unknown owners are not prose exceptions.
`SOURCES.md` selects core native compilation; installed archive proof remains
release-only.

## Check applicability

| Check | DAML-owned boundary |
|---|---|
| `docs` | Whitespace check for the positive documentation map. |
| `core` | `./scripts/check`: Rust formatting, library checks, all-feature Clippy and tests, rustdoc, and isolated Python binding checks. |
| `dependencies` | Cargo advisory, license, ban, and source checks for the native, Python-binding, and all-feature graphs. |
| `fuzz` | Fuzz harness lint, fuzz dependency policy, and bounded AddressSanitizer smoke. Full-source PRs/nightlies/manual runs retain this boundary. |
| `editors` | VS Code packaging, dependency audits, language-server build, grammar checks, and installed Electron lifecycle/semantic verification. Affected integration/semantic-editor fixtures and every full-source run select it. |
| `binaries` | Release qualification and manual `Binaries` only: native archives/installed contracts and stable-ABI Python wheels on Ubuntu 22.04 x64, macOS ARM64 and Windows x64, exercised on CPython 3.12 and 3.13. |

`src/**`, ordinary `tests/**`, and `examples/**` select `core`.
`integrations/**` and `tests/fixtures/semantic-editor/**` additionally select
`editors`. Shared lockfiles/manifests and setup changes select every source
group. A mixed docs/runtime change cannot suppress its runtime checks;
renames/deletions are classified using both sides of the complete path set.

Native archive, Python wheel and VSIX Actions artifacts retain the existing
14-day period. Automatic core checks compile the native executables on Ubuntu
24.04. Pushes do not run that gate, and automatic routes never add binary packaging.
Release qualification and manual `Binaries` preserve all three native runners,
installed archive verification and CPython 3.12/3.13 wheel consumers.
Editor verification runs in an Ubuntu Electron integration test.
See the [release/platform gate policy](../../.github/workflows/prepare-release.yml)
at the top of the qualification workflow; a green PR does not prove those checks.

Selected heavy work first checks coordinated shipping versions and the release
admission regressions. Release preparation adds browser package/type/native-parity
and Chromium verification without adding a planner identity. Collection waits for
all qualification groups, validates the unchanged eight payloads, runs corruption
probes and retains the candidate for 14 days. The canonical `Prepare release`
workflow uses this same graph; only its successful main-branch runs can be
promoted. See [release operations](releases.md).

The automatic aggregate is `Check / Required`. Keep any existing required checks
in repository rulesets and require this aggregate there; workflow YAML cannot
configure that protection policy.
