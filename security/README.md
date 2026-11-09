# Release security reference

`../scripts/release_security.py` owns this release-only gate. It uses Python's
standard library and the three immutable images in `policy.json`; it does not
install product dependencies or host scanner packages. Ordinary checks do not
invoke it. There is no scanner-selection menu.

## Invocation

The caller supplies a JSON array of admitted records, each containing exactly
`filename`, `sha256`, `size`, `kind` and `target`. Keep that input file and the
output evidence outside the source checkout and candidate directory.

```sh
python scripts/release_security.py scan --root "$SOURCE" \
  --directory "$CANDIDATE" --source-sha "$SOURCE_SHA" \
  --assets-json "$ASSETS_JSON" --output "$EVIDENCE_JSON"
python scripts/release_security.py verify --root "$SOURCE" \
  --directory "$CANDIDATE" --source-sha "$SOURCE_SHA" \
  --assets-json "$ASSETS_JSON" --evidence-file "$EVIDENCE_JSON"
```

`scan` performs image/database preparation before offline scans. It writes a new
evidence file even when scanners find issues or fail, then exits nonzero unless
all categories pass. The release collector instead calls `run_suite` directly
and embeds the result in `RELEASE.json`; no report file belongs in the bundle.
`verify` checks evidence and candidate hashes without Docker, networking or
writes. The enclosing approved manifest digest supplies provenance; this module
does not claim that a caller-written JSON document authenticates itself.

To inspect preparation independently, use a new directory outside the checkout:

```sh
python scripts/release_security.py prepare --root "$SOURCE" --cache "$NEW_CACHE"
```

That command pulls all pinned engines, checks their actual versions and image
identities, refreshes both databases and prints redacted preparation receipts.
It does not scan source or artifacts. It is diagnostic preparation, not proof of
a completed suite. A suite uses a fresh private workspace and refreshes databases
again; it does not trust receipts left by another invocation. Set
`RELEASE_SECURITY_CACHE` to an external temporary-workspace parent when needed.
The suite removes its own workspace after collecting evidence.

`SECURITY_DOCKER_CONTEXT`, then `DOCKER_CONTEXT`, selects the Docker context;
otherwise the current context is used. The module never changes shared defaults
or provisions a daemon. Only local Unix-socket contexts are admitted. Use the
existing rootless context on the authoring host; CI may use its existing daemon.

Before each default-user container invocation, read-only `docker info`
`SecurityOptions` determines the ownership mapping. Rootless execution uses
container `0:0`, which maps to the daemon owner's host identity. Rootful execution
uses the caller's effective host UID/GID, so private snapshots and cache outputs
remain accessible and removable without added capabilities or host permission
changes. This applies to metadata calls, database preparation and all scanners.
FreshClam's explicit image UID/GID override below remains separate.

The rootless pilot does not establish rootful runtime compatibility. Actual
rootful scanner smoke requires separate evidence from an authorized CI runner;
the ownership decision can also be checked in isolation without a rootful daemon.

The hosted collector uses Ubuntu 24.04 and its existing local Unix-socket daemon.
It must allow image pulls and public database downloads, provide space for the
source, eight payload copies, engine images and databases, and finish within the
collector's 30-minute job deadline. Rootful qualification must exercise all three
engines, FreshClam's UID `100:101` database writes and subsequent host cleanup.
Passing the Docker metadata/UID unit cases does not prove those runtime effects.

## Execution and disclosure boundaries

Image pulls and explicit Trivy/FreshClam preparation may use the network. Those
containers receive only a database output directory and, for FreshClam, the
minimal generated configuration. They cannot see source or candidate files.
Scanner containers use `--network=none`, read-only source/artifact/database
mounts, a read-only root filesystem and bounded temporary storage. They receive
no home, Docker socket, credential mounts, inherited authentication variables,
privileged flags or capabilities. Containers use unique names and ownership
labels, two CPUs, 4 GiB memory and 256 PIDs. Timeout and keyboard interruption
attempt to remove only that invocation's exact container name.

FreshClam runs as the official image's observed unprivileged UID/GID `100:101`,
with `DatabaseOwner clamav` and capabilities still dropped. Its public-database
leaf is temporarily mode `0777` beneath a private `0700` parent, then restored
to `0700` in `finally`. Updater-created entries receive read/write/search
permissions needed by the workspace owner for hashing and cleanup; the private
parent prevents access by other host users. No source or credentials enter this
preparation container.

Semgrep uses `scan --oss-only`, local rules, metrics off and version checks off,
including metadata calls. Trivy disables telemetry and automatic configuration,
ignore files, secret configuration and updates during scanning. There is no
registry rule fetch, AppSec login, cloud analysis, sample submission or autofix.
Raw scanner stdout/stderr is captured locally and discarded. Evidence contains
finding counts, bounded identifier/location summaries and fixed error codes.
It never contains matching source lines, secret values, unrestricted scanner
diagnostics or malware sample contents.

## Evidence schema 1

The top-level object contains exactly:

- `schema`: integer `1`.
- `source_sha`: full lowercase 40-hex Git commit.
- `assets`: exact admitted records, sorted by filename.
- `policy_sha256`, `rules_sha256`: SHA-256 of current resource bytes.
- `scanned_at`: actual completion time as UTC ISO 8601.
- `status`: `passed`, `failed` or `incomplete`.
- `checks`: exactly `dependencies`, `secrets`, `sast`, `malware`, once each.

Each check contains `id`, `status` (`passed`, `findings`, `error`), nonnegative
integer `findings`, `tool` and `coverage`. `tool` contains `name`, exact `version`,
pinned `image` and the inspected content-addressed `image_id`. Completed coverage
contains `complete: true` and category-specific scope facts. Initial errors have
`complete: false`; a late binding failure may retain completed scan coverage but
still has status `error`. Error rows contain a fixed `error` code. Failed database
preparation may also include `preparation_diagnostic`: at most 4096 printable
characters from the source-free, credential-free updater, never a source scan.
Initial failures cannot invent an actual image ID; their empty identity is never
admissible as passing proof.

Completed checks also contain `summaries` (at most 100 records) and
`summaries_truncated` (boolean). The full `findings` count is preserved when
summaries are truncated; truncation never makes a category pass. The only
summary record fields emitted are:

- Dependencies: `{id,path,package,version,severity}` using advisory identity,
  relative lock path and package identity.
- Secrets and SAST: `{id,path,line}` using rule identity and a relative scanned
  source path with a positive line number. Secret matches are never retained.
- Malware: `{id,artifact}` using signature identity and an admitted artifact name.

Identifiers are bounded token strings; locations must belong to the admitted
source inventory. Source snippets, messages and match values are not copied.
The validator permits these optional check fields in passing proof only as
`summaries: []` and `summaries_truncated: false`. Older minimal records without
either optional field remain admissible; nonpassing proof remains rejected.

An incomplete dependency check also retains earlier lock results and bounded
`failure_detail`: `{failure_count,failures,failures_truncated}`. At most 20
failures are retained. Each failure has a fixed `code`, relative lock `path` and,
when applicable, exception-class `cause`. Package omissions also include
`expected_packages`, `reported_packages`, `missing_count`, up to 20
`missing_packages` records (`{package,version}`), and `missing_truncated`.
No exception text, raw scanner JSON or source-scanner stderr is copied.
Independent locks continue after an inventory/report failure. Completed-lock
counts and advisory summaries remain visible even though the category is
incomplete and publication admission rejects it.

`dependencies.database` contains exactly `updated_at`, `downloaded_at` and the
actual `trivy.db` `sha256`. `malware.database` contains `refreshed_at` plus `daily`,
`main` and `bytecode`; each archive identity contains integer `version`,
`built_at` and `sha256`. FreshClam must succeed before `refreshed_at` is recorded.

Top-level pass requires every required category to complete with zero findings.
Any error makes the suite incomplete, even if other scanners found issues.
Independent categories continue after findings and scanner failures. A failed
advisory refresh does not prevent the available Trivy engine from checking
secrets. Publication admission rejects incomplete/failed proof, missing or extra
checks, duplicates, malformed identities and policy/source/asset mismatches.

## Binding and freshness

The source checkout must be clean and its HEAD must equal `source_sha`. Every
tracked file is copied into an isolated snapshot only after its bytes match its
Git blob identity. Source symlinks and submodules are rejected rather than
silently omitted. Scanners see this snapshot, not a live checkout; source bytes,
HEAD, clean state and current policy are checked again after scanning.

Only admitted artifacts are copied into the malware snapshot. Paths cannot
escape their candidate directory or traverse symlinks. Copied sizes and hashes
must match the admitted records, and live candidate identities are checked again
at completion. Existing archive admission remains the caller's responsibility.
Evidence never binds the final manifest hash, which would be circular.

Evidence, Trivy database build/download timestamps, successful signature refresh
and the daily signature database build must be no older than 24 hours (or a
stricter policy limit), both at completion and admission. Future timestamps are
rejected. `main` and `bytecode` archive builds may be older: FreshClam confirms
currentness, and their actual historical versions, times and hashes are recorded.
A fresh image pull or wrapper invocation cannot make an old daily database pass.

## Actual coverage and limits

- **Dependencies:** every lock listed in `policy.json`, including release,
  verification, development and fuzz tooling locks. Inventory changes fail
  closed. Cargo, npm, uv and supported pnpm v9 packages are compared with parsed
  lock identities; an unsupported/ignored lock or missing package is not a clean
  result. Cargo locks are scanned individually without manifests to retain
  development dependencies. npm and uv include development dependencies
  explicitly; the pnpm inventory includes every package entry regardless of
  importer dependency group. pnpm support is a strict inventory reader, not a
  general YAML parser: it requires lockfile version `9.0`, canonical block maps,
  semver package keys and matching package/snapshot inventories. Duplicate keys,
  YAML aliases/tags/merge keys/block scalars, unsupported top-level sections,
  non-semver keys and malformed peer contexts fail closed. Trivy still parses
  and scans the complete lockfile. This checks known advisories, not dependency
  behavior, provenance or unknown vulnerabilities. Findings across locks may
  count the same advisory repeatedly.
  A root-only npm lock can prove zero dependencies only when both its root
  record and adjacent `package.json` have no product, development, optional,
  peer or bundled dependency declarations. Trivy still scans that lock; coverage
  records zero required packages rather than inventing a package or skipping it.
  An unversioned Trivy row is excluded only when the uv lock explicitly declares
  that name as an unversioned `editable = \".\"` root, no same-named registry entry
  exists, and Trivy reports matching identity, root relationship and uv analyzer.
  Coverage names these local exclusions. An unversioned registry or unclassified
  package remains an error. Registry identities are compared exactly; no alias,
  name or version normalization fallback hides a mismatch.
- **Secrets:** Trivy's built-in current-source rules; no Git history. Trivy skips
  files under ten bytes, most binary formats, and built-in allowed paths or
  patterns. This is not a claim that every possible secret is detectable.
- **SAST:** all tracked files with the configured Python, JS/JSX, TS/TSX and Rust
  extensions, using only the local rules below. Explicit file inventory, executed
  applicable rule inventory, parse/scanner errors and skipped rules are checked.
  Language applicability comes from each local rule's adjacent `id` and
  single-line `languages: [...]` declaration and the actual target extensions.
  Every target language must have rules, at least one rule must be applicable,
  and Semgrep's reported rule IDs must match that exact applicable set.
  Python-only projects do not claim JavaScript/Rust rules executed. Coverage
  records actual `rules`, all `configured_rules` and target `languages`.
  Inline suppression is disabled. This is Semgrep CE pattern analysis, not paid
  interfile analysis or a full program-security proof.
- **Malware:** every delivered artifact, including compressed contents recognized
  by ClamAV. Missing per-artifact results, scanner errors, warnings, encrypted or
  broken inputs and maximum-limit alerts fail closed. Bounds are 1 GiB per file,
  2 GiB scanned per container file, 100,000 contained files, recursion 64 and
  300 seconds per file, plus a 30-minute command deadline. Engine format support
  and signature/normalization capabilities remain limits; passing does not mean
  malware is impossible or that unsupported formats gain semantic inspection.
- **IaC/configuration:** no deployable IaC input is configured for DAML. GitHub
  Actions YAML is not presented as Kubernetes/IaC coverage. Introducing a
  Dockerfile or Terraform input blocks the gate until reviewed coverage is
  configured; the module does not silently claim to have scanned new IaC.

## Local rule examples

These are narrow authored patterns, not an imported ruleset or claims of broad
security analysis. Positive/negative pairs for the main verification owner:

| Rule | Positive example | Negative example |
| --- | --- | --- |
| `python-subprocess-shell-true` | `subprocess.run(command, shell=True)` | `subprocess.run(["git", "status"], check=True)` |
| `python-yaml-unsafe-load` | `yaml.load(text, Loader=yaml.UnsafeLoader)` | `yaml.safe_load(text)` |
| `javascript-child-process-shell-true` | `child_process.spawn(cmd, args, {shell:true})` | `child_process.spawn(cmd, args, {shell:false})` |
| `javascript-eval-location` | `eval(window.location.hash)` | `new URLSearchParams(window.location.search)` |
| `rust-command-shell-c` | `Command::new("sh").arg("-c").arg(command)` | `Command::new("git").arg("status")` |

The shell rules flag shell interpretation, including intentionally constant
commands; they do not claim to prove untrusted data flow. The URL rule is a
direct syntactic source/sink case, not cross-function taint analysis. Rules do
not ban all subprocesses, unsafe Rust, deserialization or JavaScript evaluation.

## Upstream provenance

Flags and coverage were inspected in official documentation/source; runtime
smoke remains the integration owner's responsibility:

- [Trivy 0.76.2 filesystem CLI](https://github.com/aquasecurity/trivy/blob/v0.76.2/docs/guide/references/configuration/cli/trivy_filesystem.md)
- [Trivy 0.76.2 Python/uv coverage](https://github.com/aquasecurity/trivy/blob/v0.76.2/docs/guide/coverage/language/python.md)
- [Trivy secret analyzer exclusions](https://github.com/aquasecurity/trivy/blob/v0.76.2/pkg/fanal/analyzer/secret/secret.go)
- [Semgrep CLI reference](https://semgrep.dev/docs/cli-reference)
- [Semgrep 1.184.0 JSON schema](https://github.com/semgrep/semgrep/blob/v1.184.0/cli/src/semgrep/semgrep_interfaces/semgrep_output_v1.atd)
- [ClamAV 1.5.4 options](https://github.com/Cisco-Talos/clamav/blob/clamav-1.5.4/common/optparser.c)
- [Official ClamAV images](https://docs.clamav.net/manual/Installing/Docker.html)
